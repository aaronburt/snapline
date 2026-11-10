use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use snapline_core::ports::SecretStore as CoreSecretStore;
use snapline_core::secret::{Secret, SecretKey};
use snapline_store::settings::uploads::{Destination, Proxy, Timeout};
use snapline_store::settings::Settings;
use snapline_upload::custom::{header_secret_field, sxcu, CustomUploader};
use snapline_upload::gist::{GistUploader, TOKEN_FIELD as GIST_TOKEN_FIELD};
use snapline_upload::http::{HttpClient, HttpTimeout, Method, ProxySetting};
use snapline_upload::imgur::{ImgurUploader, CLIENT_ID_FIELD};
use snapline_upload::queue::{UploadFile, Uploader};
use snapline_upload::s3::{
    S3Config, S3Uploader, ACCESS_KEY_ID_FIELD, DEFAULT_PUBLIC_URL_PATTERN, SECRET_ACCESS_KEY_FIELD,
};
use snapline_upload::sftp::{
    HostKeyConfirmer, HostKeyPrompt, HostKeyState, HostKeyStore, SftpAuthMethod, SftpConfig,
    SftpUploader, PASSPHRASE_FIELD as SFTP_PASSPHRASE_FIELD, PASSWORD_FIELD as SFTP_PASSWORD_FIELD,
};

use crate::dto::upload::{
    DestinationActionDto, DestinationHeaderDto, DestinationInput, DestinationKindDto,
    DestinationStatusDto, DestinationViewDto,
};
use crate::uploads::DestinationSource;
use crate::{ServiceError, Services};

pub const IMGUR_NAME: &str = "Imgur";
pub const S3_NAME: &str = "Amazon S3";
pub const GIST_NAME: &str = "GitHub Gist";
pub const SFTP_NAME: &str = "SFTP";
pub const CUSTOM_NAME: &str = "Custom uploader";
pub const GIST_DETAIL: &str = "Text and code snippets";
pub const SFTP_DETAIL: &str = "Your own server over SSH";
pub const TEST_FILE_NAME: &str = "snapline-test.png";
pub const TEST_CONTENT_TYPE: &str = "image/png";
pub const TEST_GIST_FILE_NAME: &str = "snapline-test.txt";
pub const TEST_GIST_CONTENT_TYPE: &str = "text/plain";
pub const TEST_GIST_CONTENT: &str = "Snapline test upload.";
pub const HOST_KEYS_UNAVAILABLE: &str = "Host key checking is not available in this build.";

const TEST_PNG: [u8; 67] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DestinationConfig {
    Imgur {
        id: String,
    },
    S3 {
        id: String,
        endpoint: Option<String>,
        region: String,
        bucket: String,
        path_prefix: String,
        public_url_pattern: String,
        acl_public_read: bool,
    },
    Gist {
        id: String,
        public: bool,
    },
    Sftp {
        id: String,
        host: String,
        port: u16,
        username: String,
        auth_method: String,
        private_key_path: Option<String>,
        remote_folder: String,
        public_url_pattern: String,
    },
    Custom {
        id: String,
        name: String,
        method: String,
        request_url: String,
        body: String,
        file_form_field: String,
        link: String,
        thumbnail: Option<String>,
        deletion: Option<String>,
        error_message: Option<String>,
        headers: Vec<HeaderConfig>,
        imported_from_sxcu: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderConfig {
    pub name: String,
}

impl DestinationConfig {
    pub fn id(&self) -> &str {
        match self {
            Self::Imgur { id }
            | Self::S3 { id, .. }
            | Self::Gist { id, .. }
            | Self::Sftp { id, .. }
            | Self::Custom { id, .. } => id,
        }
    }

    pub fn kind(&self) -> DestinationKindDto {
        match self {
            Self::Imgur { .. } => DestinationKindDto::Imgur,
            Self::S3 { .. } => DestinationKindDto::S3,
            Self::Gist { .. } => DestinationKindDto::Gist,
            Self::Sftp { .. } => DestinationKindDto::Sftp,
            Self::Custom { .. } => DestinationKindDto::Custom,
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Imgur { .. } => IMGUR_NAME.to_owned(),
            Self::S3 { .. } => S3_NAME.to_owned(),
            Self::Gist { .. } => GIST_NAME.to_owned(),
            Self::Sftp { .. } => SFTP_NAME.to_owned(),
            Self::Custom { name, .. } => {
                if name.trim().is_empty() {
                    CUSTOM_NAME.to_owned()
                } else {
                    name.trim().to_owned()
                }
            }
        }
    }

    fn secret_fields(&self) -> Vec<String> {
        match self {
            Self::Imgur { .. } => vec![CLIENT_ID_FIELD.to_owned()],
            Self::S3 { .. } => vec![
                ACCESS_KEY_ID_FIELD.to_owned(),
                SECRET_ACCESS_KEY_FIELD.to_owned(),
            ],
            Self::Gist { .. } => vec![GIST_TOKEN_FIELD.to_owned()],
            Self::Sftp { .. } => vec![
                SFTP_PASSWORD_FIELD.to_owned(),
                SFTP_PASSPHRASE_FIELD.to_owned(),
            ],
            Self::Custom { headers, .. } => headers
                .iter()
                .map(|header| header_secret_field(&header.name))
                .collect(),
        }
    }
}

pub struct UnavailableHostKeyStore;

impl HostKeyStore for UnavailableHostKeyStore {
    fn state(
        &self,
        _host: &str,
        _port: u16,
        _key: &[u8],
    ) -> Result<HostKeyState, snapline_upload::error::UploadError> {
        Err(snapline_upload::error::UploadError::Permanent(
            HOST_KEYS_UNAVAILABLE.to_owned(),
        ))
    }

    fn remember(
        &self,
        _host: &str,
        _port: u16,
        _algorithm: &str,
        _key: &[u8],
    ) -> Result<(), snapline_upload::error::UploadError> {
        Err(snapline_upload::error::UploadError::Permanent(
            HOST_KEYS_UNAVAILABLE.to_owned(),
        ))
    }
}

pub struct UnavailableHostKeyConfirmer;

impl HostKeyConfirmer for UnavailableHostKeyConfirmer {
    fn confirm(&self, _prompt: &HostKeyPrompt) -> bool {
        false
    }
}

pub trait SxcuDialog: Send + Sync {
    fn open(&self) -> Result<Option<Vec<u8>>, ServiceError>;
    fn save(&self, suggested_name: &str, contents: &str) -> Result<Option<String>, ServiceError>;
}

struct UnavailableDialog;

impl SxcuDialog for UnavailableDialog {
    fn open(&self) -> Result<Option<Vec<u8>>, ServiceError> {
        Err(ServiceError::NotYet)
    }

    fn save(&self, _suggested_name: &str, _contents: &str) -> Result<Option<String>, ServiceError> {
        Err(ServiceError::NotYet)
    }
}

pub trait UploaderFactory: Send + Sync {
    fn build(
        &self,
        config: &DestinationConfig,
        timeout: HttpTimeout,
        proxy: ProxySetting,
    ) -> Result<Arc<dyn Uploader>, ServiceError>;
}

struct LiveFactory {
    host_keys: Arc<dyn HostKeyStore>,
    confirmer: Arc<dyn HostKeyConfirmer>,
}

impl LiveFactory {
    fn new(host_keys: Arc<dyn HostKeyStore>, confirmer: Arc<dyn HostKeyConfirmer>) -> Self {
        Self {
            host_keys,
            confirmer,
        }
    }
}

impl UploaderFactory for LiveFactory {
    fn build(
        &self,
        config: &DestinationConfig,
        timeout: HttpTimeout,
        proxy: ProxySetting,
    ) -> Result<Arc<dyn Uploader>, ServiceError> {
        match config {
            DestinationConfig::Imgur { id } => {
                let uploader = ImgurUploader::from_settings(id.clone(), timeout, proxy)
                    .map_err(upload_error)?;
                Ok(Arc::new(uploader))
            }
            DestinationConfig::S3 { .. } => {
                let s3 = s3_config(config)?;
                let uploader = S3Uploader::with_http(s3, timeout, proxy).map_err(upload_error)?;
                Ok(Arc::new(uploader))
            }
            DestinationConfig::Gist { id, public } => {
                let uploader = GistUploader::from_settings(id.clone(), *public, timeout, proxy)
                    .map_err(upload_error)?;
                Ok(Arc::new(uploader))
            }
            DestinationConfig::Sftp { .. } => {
                let sftp = sftp_config(config)?;
                Ok(Arc::new(SftpUploader::from_settings(
                    sftp,
                    timeout,
                    self.host_keys.clone(),
                    self.confirmer.clone(),
                )))
            }
            DestinationConfig::Custom { id, .. } => {
                let (custom, _) = custom_config(config)?;
                let http = HttpClient::new(timeout, proxy).map_err(upload_error)?;
                Ok(Arc::new(CustomUploader::new(
                    id.clone(),
                    custom,
                    Arc::new(http),
                )))
            }
        }
    }
}

pub trait DestinationOperations: Send + Sync {
    fn list(&self) -> Result<Vec<DestinationViewDto>, ServiceError>;
    fn add(&self, kind: DestinationKindDto) -> Result<DestinationViewDto, ServiceError>;
    fn save(&self, input: DestinationInput) -> Result<DestinationViewDto, ServiceError>;
    fn remove(&self, id: &str) -> Result<(), ServiceError>;
    fn test_uploader(&self, id: &str) -> Result<(Arc<dyn Uploader>, UploadFile), ServiceError>;
    fn import_sxcu(&self) -> Result<(Option<DestinationViewDto>, Vec<String>), ServiceError>;
    fn export_sxcu(
        &self,
        id: &str,
        include_header_values: bool,
    ) -> Result<Option<String>, ServiceError>;
}

#[derive(Clone)]
pub struct Registry {
    services: Services,
    secrets: Arc<dyn CoreSecretStore + Send + Sync>,
    dialog: Arc<dyn SxcuDialog>,
    factory: Arc<dyn UploaderFactory>,
    host_keys: Arc<dyn HostKeyStore>,
    confirmer: Arc<dyn HostKeyConfirmer>,
    custom_factory: bool,
}

impl fmt::Debug for Registry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Registry").finish_non_exhaustive()
    }
}

impl Registry {
    pub fn new(services: &Services, secrets: Arc<dyn CoreSecretStore + Send + Sync>) -> Self {
        let host_keys: Arc<dyn HostKeyStore> = Arc::new(UnavailableHostKeyStore);
        let confirmer: Arc<dyn HostKeyConfirmer> = Arc::new(UnavailableHostKeyConfirmer);
        Self {
            services: services.clone(),
            secrets,
            dialog: Arc::new(UnavailableDialog),
            factory: Arc::new(LiveFactory::new(host_keys.clone(), confirmer.clone())),
            host_keys,
            confirmer,
            custom_factory: false,
        }
    }

    pub fn with_dialog(mut self, dialog: Arc<dyn SxcuDialog>) -> Self {
        self.dialog = dialog;
        self
    }

    pub fn with_factory(mut self, factory: Arc<dyn UploaderFactory>) -> Self {
        self.factory = factory;
        self.custom_factory = true;
        self
    }

    pub fn with_sftp(
        mut self,
        host_keys: Arc<dyn HostKeyStore>,
        confirmer: Arc<dyn HostKeyConfirmer>,
    ) -> Self {
        self.host_keys = host_keys.clone();
        self.confirmer = confirmer.clone();
        if !self.custom_factory {
            self.factory = Arc::new(LiveFactory::new(host_keys, confirmer));
        }
        self
    }

    pub fn host_keys(&self) -> Arc<dyn HostKeyStore> {
        self.host_keys.clone()
    }

    pub fn host_key_confirmer(&self) -> Arc<dyn HostKeyConfirmer> {
        self.confirmer.clone()
    }

    fn settings(&self) -> Result<Settings, ServiceError> {
        self.services.settings_snapshot()
    }

    fn configs(&self) -> Result<Vec<DestinationConfig>, ServiceError> {
        let settings = self.settings()?;
        Ok(settings
            .uploads
            .destinations
            .iter()
            .filter_map(|destination| {
                toml::Value::Table(destination.extra.clone())
                    .try_into::<DestinationConfig>()
                    .ok()
            })
            .collect())
    }

    fn store_configs(&self, configs: &[DestinationConfig]) -> Result<(), ServiceError> {
        let destinations: Vec<Destination> = configs
            .iter()
            .map(|config| {
                let value = serde_json::to_value(config).map_err(|_| {
                    ServiceError::Internal("The destination could not be saved.".to_owned())
                })?;
                let table = match json_table(&value) {
                    Some(table) => table,
                    None => toml::Table::new(),
                };
                Ok(Destination { extra: table })
            })
            .collect::<Result<_, ServiceError>>()?;
        let patch = serde_json::json!({ "uploads": { "destinations": destinations } });
        let patch = serde_json::to_value(patch).map_err(|_| {
            ServiceError::Internal("The destination could not be saved.".to_owned())
        })?;
        self.services.update_settings(&patch)?;
        Ok(())
    }

    fn find(&self, id: &str) -> Result<DestinationConfig, ServiceError> {
        self.configs()?
            .into_iter()
            .find(|config| config.id() == id)
            .ok_or_else(|| {
                ServiceError::Invalid("The upload destination was not found.".to_owned())
            })
    }

    fn resolve(&self, dest: &str) -> Option<DestinationConfig> {
        let configs = self.configs().ok()?;
        if let Some(config) = configs.iter().find(|config| config.id() == dest) {
            return Some(config.clone());
        }
        configs.iter().find(|config| config.name() == dest).cloned()
    }

    fn http_settings(&self) -> Result<(HttpTimeout, ProxySetting), ServiceError> {
        let settings = self.settings()?;
        Ok((
            http_timeout(settings.uploads.timeout),
            proxy_setting(&settings),
        ))
    }

    fn secret_key(id: &str, field: &str) -> Result<SecretKey, ServiceError> {
        SecretKey::new(id, field)
            .map_err(|_| ServiceError::Invalid("The destination id is not valid.".to_owned()))
    }

    fn secret_saved(&self, id: &str, field: &str) -> bool {
        Self::secret_key(id, field)
            .ok()
            .and_then(|key| self.secrets.get(&key).ok())
            .flatten()
            .is_some()
    }

    fn put_secret(&self, id: &str, field: &str, value: &str) -> Result<(), ServiceError> {
        let key = Self::secret_key(id, field)?;
        let secret = Secret::from(value);
        self.secrets
            .put(&key, &secret)
            .map_err(|_| ServiceError::Internal("The secret could not be saved.".to_owned()))
    }

    fn delete_secret(&self, id: &str, field: &str) {
        if let Ok(key) = Self::secret_key(id, field) {
            let _ = self.secrets.delete(&key);
        }
    }

    fn next_id(&self, kind: DestinationKindDto, configs: &[DestinationConfig]) -> String {
        let base = match kind {
            DestinationKindDto::Imgur => "imgur",
            DestinationKindDto::S3 => "s3",
            DestinationKindDto::Gist => "gist",
            DestinationKindDto::Sftp => "sftp",
            DestinationKindDto::Custom => "custom",
        };
        if !configs.iter().any(|config| config.id() == base) {
            return base.to_owned();
        }
        let mut number = 2_u32;
        loop {
            let candidate = format!("{base}-{number}");
            if !configs.iter().any(|config| config.id() == candidate) {
                return candidate;
            }
            number += 1;
        }
    }

    fn view(&self, config: &DestinationConfig) -> DestinationViewDto {
        let warnings = match config {
            DestinationConfig::Custom { .. } => custom_warnings(config),
            _ => Vec::new(),
        };
        match config {
            DestinationConfig::Imgur { id } => {
                let saved = self.secret_saved(id, CLIENT_ID_FIELD);
                DestinationViewDto {
                    id: id.clone(),
                    kind: DestinationKindDto::Imgur,
                    name: IMGUR_NAME.to_owned(),
                    detail: "Image host · anonymous uploads".to_owned(),
                    status: status(saved),
                    action: DestinationActionDto::Configure,
                    configured: saved,
                    client_id_saved: saved,
                    endpoint: None,
                    region: None,
                    bucket: None,
                    path_prefix: None,
                    public_url_pattern: None,
                    acl_public_read: false,
                    access_key_id_saved: false,
                    secret_access_key_saved: false,
                    method: None,
                    request_url: None,
                    body: None,
                    file_form_field: None,
                    link: None,
                    thumbnail: None,
                    deletion: None,
                    error_message: None,
                    headers: Vec::new(),
                    imported_from_sxcu: false,
                    host: None,
                    port: None,
                    username: None,
                    auth_method: None,
                    private_key_path: None,
                    remote_folder: None,
                    password_saved: false,
                    passphrase_saved: false,
                    warnings,
                }
            }
            DestinationConfig::S3 {
                id,
                endpoint,
                region,
                bucket,
                path_prefix,
                public_url_pattern,
                acl_public_read,
            } => {
                let access = self.secret_saved(id, ACCESS_KEY_ID_FIELD);
                let secret = self.secret_saved(id, SECRET_ACCESS_KEY_FIELD);
                let configured = !bucket.trim().is_empty()
                    && !region.trim().is_empty()
                    && access
                    && secret
                    && endpoint_valid(endpoint.as_deref())
                    && public_url_pattern.contains("<key>");
                DestinationViewDto {
                    id: id.clone(),
                    kind: DestinationKindDto::S3,
                    name: S3_NAME.to_owned(),
                    detail: if bucket.trim().is_empty() {
                        "Bucket not set".to_owned()
                    } else {
                        format!("Bucket {} · {}", bucket.trim(), region.trim())
                    },
                    status: status(configured),
                    action: if configured {
                        DestinationActionDto::Configure
                    } else {
                        DestinationActionDto::SetUp
                    },
                    configured,
                    client_id_saved: false,
                    endpoint: endpoint.clone(),
                    region: Some(region.clone()),
                    bucket: Some(bucket.clone()),
                    path_prefix: Some(path_prefix.clone()),
                    public_url_pattern: Some(public_url_pattern.clone()),
                    acl_public_read: *acl_public_read,
                    access_key_id_saved: access,
                    secret_access_key_saved: secret,
                    method: None,
                    request_url: None,
                    body: None,
                    file_form_field: None,
                    link: None,
                    thumbnail: None,
                    deletion: None,
                    error_message: None,
                    headers: Vec::new(),
                    imported_from_sxcu: false,
                    host: None,
                    port: None,
                    username: None,
                    auth_method: None,
                    private_key_path: None,
                    remote_folder: None,
                    password_saved: false,
                    passphrase_saved: false,
                    warnings,
                }
            }
            DestinationConfig::Gist { id, public } => {
                let saved = self.secret_saved(id, GIST_TOKEN_FIELD);
                DestinationViewDto {
                    id: id.clone(),
                    kind: DestinationKindDto::Gist,
                    name: GIST_NAME.to_owned(),
                    detail: GIST_DETAIL.to_owned(),
                    status: status(saved),
                    action: if saved {
                        DestinationActionDto::Configure
                    } else {
                        DestinationActionDto::SetUp
                    },
                    configured: saved,
                    client_id_saved: saved,
                    endpoint: None,
                    region: None,
                    bucket: None,
                    path_prefix: None,
                    public_url_pattern: None,
                    acl_public_read: *public,
                    access_key_id_saved: false,
                    secret_access_key_saved: false,
                    method: None,
                    request_url: None,
                    body: None,
                    file_form_field: None,
                    link: None,
                    thumbnail: None,
                    deletion: None,
                    error_message: None,
                    headers: Vec::new(),
                    imported_from_sxcu: false,
                    host: None,
                    port: None,
                    username: None,
                    auth_method: None,
                    private_key_path: None,
                    remote_folder: None,
                    password_saved: false,
                    passphrase_saved: false,
                    warnings,
                }
            }
            DestinationConfig::Sftp {
                id,
                host,
                port,
                username,
                auth_method,
                private_key_path,
                remote_folder,
                public_url_pattern,
            } => {
                let auth = SftpAuthMethod::parse(auth_method).unwrap_or(SftpAuthMethod::Password);
                let password_saved = self.secret_saved(id, SFTP_PASSWORD_FIELD);
                let passphrase_saved = self.secret_saved(id, SFTP_PASSPHRASE_FIELD);
                let key_saved = private_key_path
                    .as_deref()
                    .map(str::trim)
                    .map(|path| !path.is_empty())
                    .unwrap_or(false);
                let configured = !host.trim().is_empty()
                    && *port != 0
                    && !username.trim().is_empty()
                    && public_url_pattern.contains("<key>")
                    && match auth {
                        SftpAuthMethod::Password => password_saved,
                        SftpAuthMethod::PrivateKey => key_saved,
                    };
                DestinationViewDto {
                    id: id.clone(),
                    kind: DestinationKindDto::Sftp,
                    name: SFTP_NAME.to_owned(),
                    detail: SFTP_DETAIL.to_owned(),
                    status: status(configured),
                    action: if configured {
                        DestinationActionDto::Configure
                    } else {
                        DestinationActionDto::SetUp
                    },
                    configured,
                    client_id_saved: false,
                    endpoint: None,
                    region: None,
                    bucket: None,
                    path_prefix: None,
                    public_url_pattern: Some(public_url_pattern.clone()),
                    acl_public_read: false,
                    access_key_id_saved: false,
                    secret_access_key_saved: false,
                    method: None,
                    request_url: None,
                    body: None,
                    file_form_field: None,
                    link: None,
                    thumbnail: None,
                    deletion: None,
                    error_message: None,
                    headers: Vec::new(),
                    imported_from_sxcu: false,
                    host: Some(host.clone()),
                    port: Some(*port),
                    username: Some(username.clone()),
                    auth_method: Some(auth.as_str().to_owned()),
                    private_key_path: private_key_path.clone(),
                    remote_folder: Some(remote_folder.clone()),
                    password_saved,
                    passphrase_saved,
                    warnings,
                }
            }
            DestinationConfig::Custom {
                id,
                name,
                method,
                request_url,
                body,
                file_form_field,
                link,
                thumbnail,
                deletion,
                error_message,
                headers,
                imported_from_sxcu,
            } => {
                let configured = custom_config(config).is_ok();
                DestinationViewDto {
                    id: id.clone(),
                    kind: DestinationKindDto::Custom,
                    name: if name.trim().is_empty() {
                        CUSTOM_NAME.to_owned()
                    } else {
                        name.trim().to_owned()
                    },
                    detail: if *imported_from_sxcu {
                        "Custom uploader · imported from .sxcu".to_owned()
                    } else if name.trim().is_empty() || name.trim() == CUSTOM_NAME {
                        CUSTOM_NAME.to_owned()
                    } else {
                        format!("Custom uploader · {}", name.trim())
                    },
                    status: status(configured),
                    action: if configured {
                        DestinationActionDto::Edit
                    } else {
                        DestinationActionDto::SetUp
                    },
                    configured,
                    client_id_saved: false,
                    endpoint: None,
                    region: None,
                    bucket: None,
                    path_prefix: None,
                    public_url_pattern: None,
                    acl_public_read: false,
                    access_key_id_saved: false,
                    secret_access_key_saved: false,
                    method: Some(method.clone()),
                    request_url: Some(request_url.clone()),
                    body: Some(body.clone()),
                    file_form_field: Some(file_form_field.clone()),
                    link: Some(link.clone()),
                    thumbnail: thumbnail.clone(),
                    deletion: deletion.clone(),
                    error_message: error_message.clone(),
                    headers: headers
                        .iter()
                        .map(|header| DestinationHeaderDto {
                            name: header.name.clone(),
                            saved: self.secret_saved(id, &header_secret_field(&header.name)),
                        })
                        .collect(),
                    imported_from_sxcu: *imported_from_sxcu,
                    host: None,
                    port: None,
                    username: None,
                    auth_method: None,
                    private_key_path: None,
                    remote_folder: None,
                    password_saved: false,
                    passphrase_saved: false,
                    warnings,
                }
            }
        }
    }
}

impl DestinationSource for Registry {
    fn uploader(&self, dest: &str) -> Option<Arc<dyn Uploader>> {
        let config = self.resolve(dest)?;
        let (timeout, proxy) = self.http_settings().ok()?;
        self.factory.build(&config, timeout, proxy).ok()
    }

    fn streams_files(&self, dest: &str) -> bool {
        matches!(self.resolve(dest), Some(DestinationConfig::Sftp { .. }))
    }

    fn operations(&self) -> Option<Arc<dyn DestinationOperations>> {
        Some(Arc::new(self.clone()))
    }
}

impl DestinationOperations for Registry {
    fn list(&self) -> Result<Vec<DestinationViewDto>, ServiceError> {
        Ok(self
            .configs()?
            .iter()
            .map(|config| self.view(config))
            .collect())
    }

    fn add(&self, kind: DestinationKindDto) -> Result<DestinationViewDto, ServiceError> {
        let mut configs = self.configs()?;
        let id = self.next_id(kind, &configs);
        let config = match kind {
            DestinationKindDto::Imgur => DestinationConfig::Imgur { id },
            DestinationKindDto::S3 => DestinationConfig::S3 {
                id,
                endpoint: None,
                region: String::new(),
                bucket: String::new(),
                path_prefix: String::new(),
                public_url_pattern: DEFAULT_PUBLIC_URL_PATTERN.to_owned(),
                acl_public_read: false,
            },
            DestinationKindDto::Gist => DestinationConfig::Gist { id, public: false },
            DestinationKindDto::Sftp => DestinationConfig::Sftp {
                id,
                host: String::new(),
                port: snapline_upload::sftp::DEFAULT_PORT,
                username: String::new(),
                auth_method: SftpAuthMethod::Password.as_str().to_owned(),
                private_key_path: None,
                remote_folder: String::new(),
                public_url_pattern: String::new(),
            },
            DestinationKindDto::Custom => DestinationConfig::Custom {
                id,
                name: CUSTOM_NAME.to_owned(),
                method: "POST".to_owned(),
                request_url: String::new(),
                body: "MultipartFormData".to_owned(),
                file_form_field: "file".to_owned(),
                link: String::new(),
                thumbnail: None,
                deletion: None,
                error_message: None,
                headers: Vec::new(),
                imported_from_sxcu: false,
            },
        };
        let view = self.view(&config);
        configs.push(config);
        self.store_configs(&configs)?;
        Ok(view)
    }

    fn save(&self, input: DestinationInput) -> Result<DestinationViewDto, ServiceError> {
        let mut configs = self.configs()?;
        let id = input
            .id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| {
                ServiceError::Invalid("The upload destination was not found.".to_owned())
            })?;
        let position = configs
            .iter()
            .position(|config| config.id() == id)
            .ok_or_else(|| {
                ServiceError::Invalid("The upload destination was not found.".to_owned())
            })?;
        let existing = configs[position].clone();
        if existing.kind() != input.kind {
            return Err(ServiceError::Invalid(
                "The upload destination type cannot be changed.".to_owned(),
            ));
        }
        let config = match existing {
            DestinationConfig::Imgur { id } => {
                let saved = self.secret_saved(&id, CLIENT_ID_FIELD);
                let client_id = input
                    .client_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(client_id) = client_id {
                    if !client_id.bytes().all(|byte| byte.is_ascii_graphic()) {
                        return Err(ServiceError::Invalid(
                            "The Client ID is not valid.".to_owned(),
                        ));
                    }
                    self.put_secret(&id, CLIENT_ID_FIELD, client_id)?;
                } else if !saved {
                    return Err(ServiceError::Invalid(
                        "Enter the Imgur Client ID.".to_owned(),
                    ));
                }
                DestinationConfig::Imgur { id }
            }
            DestinationConfig::S3 { id, .. } => {
                let region = input.region.as_deref().unwrap_or("").trim().to_owned();
                let bucket = input.bucket.as_deref().unwrap_or("").trim().to_owned();
                if region.is_empty() {
                    return Err(ServiceError::Invalid("Enter the S3 region.".to_owned()));
                }
                if bucket.is_empty() {
                    return Err(ServiceError::Invalid("Enter the S3 bucket.".to_owned()));
                }
                let endpoint = input
                    .endpoint
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                if !endpoint_valid(endpoint.as_deref()) {
                    return Err(ServiceError::Invalid(
                        "The endpoint must be a full http:// or https:// address.".to_owned(),
                    ));
                }
                let pattern = input
                    .public_url_pattern
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(DEFAULT_PUBLIC_URL_PATTERN)
                    .to_owned();
                if !pattern.contains("<key>") {
                    return Err(ServiceError::Invalid(
                        "The public URL pattern must contain <key>.".to_owned(),
                    ));
                }
                let access = input
                    .access_key_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let secret = input
                    .secret_access_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let access_saved = self.secret_saved(&id, ACCESS_KEY_ID_FIELD);
                let secret_saved = self.secret_saved(&id, SECRET_ACCESS_KEY_FIELD);
                if access.is_none() && !access_saved {
                    return Err(ServiceError::Invalid(
                        "Enter the S3 access key ID.".to_owned(),
                    ));
                }
                if secret.is_none() && !secret_saved {
                    return Err(ServiceError::Invalid(
                        "Enter the S3 secret access key.".to_owned(),
                    ));
                }
                if let Some(access) = access {
                    self.put_secret(&id, ACCESS_KEY_ID_FIELD, access)?;
                }
                if let Some(secret) = secret {
                    self.put_secret(&id, SECRET_ACCESS_KEY_FIELD, secret)?;
                }
                DestinationConfig::S3 {
                    id,
                    endpoint,
                    region,
                    bucket,
                    path_prefix: input.path_prefix.as_deref().unwrap_or("").trim().to_owned(),
                    public_url_pattern: pattern,
                    acl_public_read: input.acl_public_read.unwrap_or(false),
                }
            }
            DestinationConfig::Gist { id, .. } => {
                let saved = self.secret_saved(&id, GIST_TOKEN_FIELD);
                let token = input
                    .client_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(token) = token {
                    if !token.bytes().all(|byte| byte.is_ascii_graphic()) {
                        return Err(ServiceError::Invalid(
                            "The GitHub token is not valid.".to_owned(),
                        ));
                    }
                    self.put_secret(&id, GIST_TOKEN_FIELD, token)?;
                } else if !saved {
                    return Err(ServiceError::Invalid("Enter the GitHub token.".to_owned()));
                }
                DestinationConfig::Gist {
                    id,
                    public: input.acl_public_read.unwrap_or(false),
                }
            }
            DestinationConfig::Sftp {
                id, auth_method, ..
            } => {
                let host = input.host.as_deref().unwrap_or("").trim().to_owned();
                if host.is_empty() {
                    return Err(ServiceError::Invalid("Enter the SFTP host.".to_owned()));
                }
                let port = input.port.unwrap_or(snapline_upload::sftp::DEFAULT_PORT);
                if port == 0 {
                    return Err(ServiceError::Invalid(
                        "The SFTP port must be between 1 and 65535.".to_owned(),
                    ));
                }
                let username = input.username.as_deref().unwrap_or("").trim().to_owned();
                if username.is_empty() {
                    return Err(ServiceError::Invalid("Enter the SFTP username.".to_owned()));
                }
                let previous =
                    SftpAuthMethod::parse(&auth_method).unwrap_or(SftpAuthMethod::Password);
                let auth = match input.auth_method.as_deref() {
                    Some(value) => SftpAuthMethod::parse(value).ok_or_else(|| {
                        ServiceError::Invalid(
                            "The SFTP authentication method is not valid.".to_owned(),
                        )
                    })?,
                    None => previous,
                };
                let pattern = input
                    .public_url_pattern
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if !pattern.contains("<key>") {
                    return Err(ServiceError::Invalid(
                        "The public URL pattern must contain <key>.".to_owned(),
                    ));
                }
                let private_key_path = input
                    .private_key_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                if auth == SftpAuthMethod::PrivateKey && private_key_path.is_none() {
                    return Err(ServiceError::Invalid(
                        "Choose the SFTP private key file.".to_owned(),
                    ));
                }
                if auth == SftpAuthMethod::Password {
                    let saved = self.secret_saved(&id, SFTP_PASSWORD_FIELD);
                    let password = input
                        .password
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    if password.is_none() && !saved {
                        return Err(ServiceError::Invalid("Enter the SFTP password.".to_owned()));
                    }
                    if let Some(password) = password {
                        self.put_secret(&id, SFTP_PASSWORD_FIELD, password)?;
                    }
                } else {
                    let passphrase = input
                        .passphrase
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty());
                    if let Some(passphrase) = passphrase {
                        self.put_secret(&id, SFTP_PASSPHRASE_FIELD, passphrase)?;
                    }
                }
                if auth != previous {
                    match auth {
                        SftpAuthMethod::Password => {
                            self.delete_secret(&id, SFTP_PASSPHRASE_FIELD);
                        }
                        SftpAuthMethod::PrivateKey => {
                            self.delete_secret(&id, SFTP_PASSWORD_FIELD);
                        }
                    }
                }
                DestinationConfig::Sftp {
                    id,
                    host,
                    port,
                    username,
                    auth_method: auth.as_str().to_owned(),
                    private_key_path: if auth == SftpAuthMethod::PrivateKey {
                        private_key_path
                    } else {
                        None
                    },
                    remote_folder: input
                        .remote_folder
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .to_owned(),
                    public_url_pattern: pattern,
                }
            }
            DestinationConfig::Custom {
                id,
                headers: old_headers,
                imported_from_sxcu,
                ..
            } => {
                let name = input
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(CUSTOM_NAME)
                    .to_owned();
                let method = input
                    .method
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("POST")
                    .to_owned();
                let body = input
                    .body
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("MultipartFormData")
                    .to_owned();
                let request_url = input.request_url.as_deref().unwrap_or("").trim().to_owned();
                let file_form_field = input
                    .file_form_field
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                let link = input.link.as_deref().unwrap_or("").trim().to_owned();
                let headers = input
                    .headers
                    .iter()
                    .filter(|header| !header.name.trim().is_empty())
                    .map(|header| HeaderConfig {
                        name: header.name.trim().to_owned(),
                    })
                    .collect::<Vec<_>>();
                if headers.is_empty()
                    && input
                        .headers
                        .iter()
                        .any(|header| header.name.trim().is_empty() && header.value.is_some())
                {
                    return Err(ServiceError::Invalid(
                        "A header name is required.".to_owned(),
                    ));
                }
                let config = DestinationConfig::Custom {
                    id: id.clone(),
                    name,
                    method,
                    request_url,
                    body,
                    file_form_field,
                    link,
                    thumbnail: optional_text(input.thumbnail.as_deref()),
                    deletion: optional_text(input.deletion.as_deref()),
                    error_message: optional_text(input.error_message.as_deref()),
                    headers,
                    imported_from_sxcu: input.imported_from_sxcu.unwrap_or(imported_from_sxcu),
                };
                custom_config(&config)?;
                for header in &input.headers {
                    let name = header.name.trim();
                    if name.is_empty() {
                        continue;
                    }
                    if let Some(value) = header
                        .value
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        self.put_secret(&id, &header_secret_field(name), value)?;
                    }
                }
                let kept: Vec<String> = config
                    .secret_fields()
                    .into_iter()
                    .map(|field| field.to_ascii_lowercase())
                    .collect();
                for header in old_headers {
                    let field = header_secret_field(&header.name);
                    if !kept.contains(&field.to_ascii_lowercase()) {
                        self.delete_secret(&id, &field);
                    }
                }
                config
            }
        };
        let view = self.view(&config);
        configs[position] = config;
        self.store_configs(&configs)?;
        Ok(view)
    }

    fn remove(&self, id: &str) -> Result<(), ServiceError> {
        let mut configs = self.configs()?;
        let position = configs
            .iter()
            .position(|config| config.id() == id)
            .ok_or_else(|| {
                ServiceError::Invalid("The upload destination was not found.".to_owned())
            })?;
        let removed = configs.remove(position);
        for field in removed.secret_fields() {
            self.delete_secret(removed.id(), &field);
        }
        self.store_configs(&configs)
    }

    fn test_uploader(&self, id: &str) -> Result<(Arc<dyn Uploader>, UploadFile), ServiceError> {
        let config = self.find(id)?;
        if !self.view(&config).configured {
            return Err(ServiceError::Invalid(
                "Finish setting up this destination first.".to_owned(),
            ));
        }
        let (timeout, proxy) = self.http_settings()?;
        let uploader = self.factory.build(&config, timeout, proxy)?;
        Ok((uploader, test_file_for(&config)))
    }

    fn import_sxcu(&self) -> Result<(Option<DestinationViewDto>, Vec<String>), ServiceError> {
        let Some(bytes) = self.dialog.open()? else {
            return Ok((None, Vec::new()));
        };
        let imported = sxcu::import(&bytes).map_err(sxcu_error)?;
        let mut configs = self.configs()?;
        let id = self.next_id(DestinationKindDto::Custom, &configs);
        let method = imported.config.method.as_str().to_owned();
        let body = imported.config.body.as_sharex().to_owned();
        let headers = imported
            .config
            .headers
            .iter()
            .map(|header| HeaderConfig {
                name: header.name.clone(),
            })
            .collect::<Vec<_>>();
        let config = DestinationConfig::Custom {
            id: id.clone(),
            name: imported.config.name.clone(),
            method,
            request_url: imported.config.request_url.clone(),
            body,
            file_form_field: imported.config.file_form_field.clone(),
            link: imported.config.link.clone(),
            thumbnail: imported.config.thumbnail.clone(),
            deletion: imported.config.deletion.clone(),
            error_message: imported.config.error_message.clone(),
            headers,
            imported_from_sxcu: true,
        };
        for header in &imported.config.headers {
            if let Some(value) = imported.header_values.get(&header.name) {
                if !value.is_empty() {
                    self.put_secret(&id, &header_secret_field(&header.name), value)?;
                }
            }
        }
        let warnings = imported
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let view = self.view(&config);
        configs.push(config);
        self.store_configs(&configs)?;
        Ok((Some(view), warnings))
    }

    fn export_sxcu(
        &self,
        id: &str,
        include_header_values: bool,
    ) -> Result<Option<String>, ServiceError> {
        let config = self.find(id)?;
        let DestinationConfig::Custom { name, .. } = &config else {
            return Err(ServiceError::Invalid(
                "Only custom uploaders can be exported.".to_owned(),
            ));
        };
        let (custom, _) = custom_config(&config)?;
        let mut values = sxcu::HeaderValues::new();
        if include_header_values {
            for header in &custom.headers {
                if let Ok(key) = Self::secret_key(id, &header_secret_field(&header.name)) {
                    if let Ok(Some(secret)) = self.secrets.get(&key) {
                        if let Ok(value) = std::str::from_utf8(secret.expose()) {
                            values.insert(header.name.clone(), value);
                        }
                    }
                }
            }
        }
        let text = sxcu::export(&custom, include_header_values, &values).map_err(sxcu_error)?;
        if text.len() > sxcu::MAX_FILE_BYTES {
            return Err(ServiceError::Invalid(
                "The .sxcu file is larger than 64 KiB.".to_owned(),
            ));
        }
        let file_name = format!("{}.sxcu", safe_file_name(name));
        self.dialog.save(&file_name, &text)
    }
}

fn status(configured: bool) -> DestinationStatusDto {
    if configured {
        DestinationStatusDto::Configured
    } else {
        DestinationStatusDto::NotSetUp
    }
}

fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn http_timeout(timeout: Timeout) -> HttpTimeout {
    match timeout {
        Timeout::Secs30 => HttpTimeout::ThirtySeconds,
        Timeout::Secs60 => HttpTimeout::SixtySeconds,
        Timeout::Secs120 => HttpTimeout::OneHundredTwentySeconds,
    }
}

fn proxy_setting(settings: &Settings) -> ProxySetting {
    match settings.uploads.proxy {
        Proxy::System => ProxySetting::System,
        Proxy::None => ProxySetting::None,
        Proxy::Custom => settings
            .uploads
            .proxy_url
            .clone()
            .filter(|url| !url.trim().is_empty())
            .map(ProxySetting::Custom)
            .unwrap_or(ProxySetting::None),
    }
}

fn upload_error(error: snapline_upload::error::UploadError) -> ServiceError {
    ServiceError::Invalid(error.to_string())
}

fn sxcu_error(error: sxcu::SxcuError) -> ServiceError {
    ServiceError::Invalid(error.to_string())
}

fn json_table(value: &serde_json::Value) -> Option<toml::Table> {
    let object = value.as_object()?;
    let mut table = toml::Table::new();
    for (key, value) in object {
        if let Some(value) = json_value(value) {
            table.insert(key.clone(), value);
        }
    }
    Some(table)
}

fn json_value(value: &serde_json::Value) -> Option<toml::Value> {
    match value {
        serde_json::Value::Bool(value) => Some(toml::Value::Boolean(*value)),
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| number.as_f64().map(toml::Value::Float)),
        serde_json::Value::String(value) => Some(toml::Value::String(value.clone())),
        serde_json::Value::Array(items) => Some(toml::Value::Array(
            items.iter().filter_map(json_value).collect(),
        )),
        serde_json::Value::Object(_) => json_table(value).map(toml::Value::Table),
        serde_json::Value::Null => None,
    }
}

fn endpoint_valid(endpoint: Option<&str>) -> bool {
    let Some(endpoint) = endpoint else {
        return true;
    };
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return true;
    }
    let lower = endpoint.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    endpoint.split_once("://").map_or(false, |(_, rest)| {
        !rest.is_empty() && !rest.starts_with('/')
    })
}

fn method_value(value: &str) -> Result<Method, ServiceError> {
    match value.trim().to_ascii_uppercase().as_str() {
        "POST" | "" => Ok(Method::Post),
        "PUT" => Ok(Method::Put),
        "PATCH" => Ok(Method::Patch),
        _ => Err(ServiceError::Invalid(
            "The request method must be POST, PUT or PATCH.".to_owned(),
        )),
    }
}

fn body_value(value: &str) -> Result<sxcu::BodyType, ServiceError> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("MultipartFormData") {
        Ok(sxcu::BodyType::MultipartFormData)
    } else if value.eq_ignore_ascii_case("Binary") {
        Ok(sxcu::BodyType::Binary)
    } else if value.eq_ignore_ascii_case("FormURLEncoded") {
        Ok(sxcu::BodyType::FormUrlEncoded)
    } else if value.eq_ignore_ascii_case("JSON") {
        Ok(sxcu::BodyType::Json)
    } else {
        Err(ServiceError::Invalid(
            "The body must be Multipart form data, Binary, Form URL-encoded or JSON.".to_owned(),
        ))
    }
}

fn s3_config(config: &DestinationConfig) -> Result<S3Config, ServiceError> {
    let DestinationConfig::S3 {
        id,
        endpoint,
        region,
        bucket,
        path_prefix,
        public_url_pattern,
        acl_public_read,
    } = config
    else {
        return Err(ServiceError::Invalid(
            "The upload destination is not an S3 bucket.".to_owned(),
        ));
    };
    let mut s3 = S3Config::new(id.clone(), bucket.clone(), region.clone())
        .path_prefix(path_prefix.clone())
        .public_url_pattern(public_url_pattern.clone())
        .acl_public_read(*acl_public_read);
    if let Some(endpoint) = endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        s3 = s3.endpoint(endpoint);
    }
    Ok(s3)
}

fn sftp_config(config: &DestinationConfig) -> Result<SftpConfig, ServiceError> {
    let DestinationConfig::Sftp {
        id,
        host,
        port,
        username,
        auth_method,
        private_key_path,
        remote_folder,
        public_url_pattern,
    } = config
    else {
        return Err(ServiceError::Invalid(
            "The upload destination is not an SFTP server.".to_owned(),
        ));
    };
    let auth = SftpAuthMethod::parse(auth_method).ok_or_else(|| {
        ServiceError::Invalid("The SFTP authentication method is not valid.".to_owned())
    })?;
    let mut sftp = SftpConfig::new(id.clone(), host.clone(), username.clone())
        .port(*port)
        .auth(auth)
        .remote_folder(remote_folder.clone())
        .public_url_pattern(public_url_pattern.clone());
    if let Some(path) = private_key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sftp = sftp.private_key_path(path);
    }
    Ok(sftp)
}

fn custom_config(
    config: &DestinationConfig,
) -> Result<(sxcu::CustomUploaderConfig, Vec<String>), ServiceError> {
    let DestinationConfig::Custom {
        name,
        method,
        request_url,
        body,
        file_form_field,
        link,
        thumbnail,
        deletion,
        error_message,
        headers,
        ..
    } = config
    else {
        return Err(ServiceError::Invalid(
            "The upload destination is not a custom uploader.".to_owned(),
        ));
    };
    let custom = sxcu::CustomUploaderConfig {
        name: name.clone(),
        method: method_value(method)?,
        request_url: request_url.clone(),
        body: body_value(body)?,
        file_form_field: file_form_field.clone(),
        link: link.clone(),
        thumbnail: thumbnail.clone(),
        deletion: deletion.clone(),
        error_message: error_message.clone(),
        headers: headers
            .iter()
            .map(|header| sxcu::Header::new(header.name.clone(), true))
            .collect(),
    };
    let warnings = sxcu::validate_config(&custom)
        .map_err(sxcu_error)?
        .iter()
        .map(ToString::to_string)
        .collect();
    Ok((custom, warnings))
}

fn custom_warnings(config: &DestinationConfig) -> Vec<String> {
    match custom_config(config) {
        Ok((_, warnings)) => warnings,
        Err(error) => vec![error.to_string()],
    }
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ' ' | '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "uploader".to_owned()
    } else {
        cleaned.to_owned()
    }
}

pub fn test_file() -> UploadFile {
    UploadFile::new(
        std::path::PathBuf::from(TEST_FILE_NAME),
        TEST_FILE_NAME,
        TEST_CONTENT_TYPE,
        TEST_PNG.to_vec(),
    )
}

fn test_file_for(config: &DestinationConfig) -> UploadFile {
    if matches!(config, DestinationConfig::Gist { .. }) {
        return UploadFile::new(
            std::path::PathBuf::from(TEST_GIST_FILE_NAME),
            TEST_GIST_FILE_NAME,
            TEST_GIST_CONTENT_TYPE,
            TEST_GIST_CONTENT.as_bytes().to_vec(),
        );
    }
    test_file()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use snapline_core::ports::SecretError;
    use snapline_core::secret::Secret;
    use snapline_store::io::Paths;
    use snapline_upload::queue::{DeletionLink, Uploaded};
    use tempfile::TempDir;

    use super::*;
    use crate::dto::upload::{
        DestinationHeaderInput, DestinationReq, DestinationTestStatusDto, DestinationViewDto,
        UploadReq, UploadResp,
    };
    use crate::uploads::{DestinationSource, FileSource, UploaderThread};

    #[derive(Default)]
    struct FakeSecrets {
        values: Mutex<HashMap<(String, String), Vec<u8>>>,
    }

    impl FakeSecrets {
        fn has(&self, id: &str, field: &str) -> bool {
            self.values
                .lock()
                .expect("secrets")
                .contains_key(&(id.to_owned(), field.to_owned()))
        }
    }

    impl CoreSecretStore for FakeSecrets {
        fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
            Ok(self
                .values
                .lock()
                .expect("secrets")
                .get(&(key.dest_id().to_owned(), key.field().to_owned()))
                .map(|value| Secret::from(value.clone())))
        }

        fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError> {
            self.values.lock().expect("secrets").insert(
                (key.dest_id().to_owned(), key.field().to_owned()),
                value.expose().to_vec(),
            );
            Ok(())
        }

        fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
            self.values
                .lock()
                .expect("secrets")
                .remove(&(key.dest_id().to_owned(), key.field().to_owned()));
            Ok(())
        }
    }

    struct FakeDialog {
        opened: Mutex<Option<Vec<u8>>>,
        saved: Mutex<Vec<(String, String)>>,
    }

    impl FakeDialog {
        fn with_file(bytes: Vec<u8>) -> Self {
            Self {
                opened: Mutex::new(Some(bytes)),
                saved: Mutex::new(Vec::new()),
            }
        }

        fn empty() -> Self {
            Self {
                opened: Mutex::new(None),
                saved: Mutex::new(Vec::new()),
            }
        }
    }

    impl SxcuDialog for FakeDialog {
        fn open(&self) -> Result<Option<Vec<u8>>, ServiceError> {
            Ok(self.opened.lock().expect("dialog").clone())
        }

        fn save(
            &self,
            suggested_name: &str,
            contents: &str,
        ) -> Result<Option<String>, ServiceError> {
            self.saved
                .lock()
                .expect("dialog")
                .push((suggested_name.to_owned(), contents.to_owned()));
            Ok(Some(format!("C:\\Users\\Test\\{suggested_name}")))
        }
    }

    struct FakeUploader {
        calls: AtomicU32,
    }

    impl FakeUploader {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicU32::new(0),
            })
        }
    }

    impl Uploader for FakeUploader {
        fn upload(
            &self,
            file: &UploadFile,
            _secrets: &dyn snapline_upload::queue::SecretStore,
        ) -> Result<Uploaded, snapline_upload::error::UploadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Uploaded::new(format!(
                "https://example.test/{}",
                file.file_name
            )))
        }

        fn delete(
            &self,
            _link: &DeletionLink,
            _secrets: &dyn snapline_upload::queue::SecretStore,
        ) -> Result<(), snapline_upload::error::UploadError> {
            Ok(())
        }
    }

    struct FakeFactory {
        uploader: Arc<FakeUploader>,
    }

    impl UploaderFactory for FakeFactory {
        fn build(
            &self,
            _config: &DestinationConfig,
            _timeout: HttpTimeout,
            _proxy: ProxySetting,
        ) -> Result<Arc<dyn Uploader>, ServiceError> {
            Ok(self.uploader.clone())
        }
    }

    struct NoFiles;

    impl FileSource for NoFiles {
        fn file(&self, _capture_id: i64) -> Result<Option<UploadFile>, ServiceError> {
            Ok(None)
        }
    }

    struct Fixture {
        _directory: TempDir,
        services: Services,
        paths: Paths,
        secrets: Arc<FakeSecrets>,
    }

    fn fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("temp directory");
        let root = directory.path();
        let paths = Paths::resolve(
            &root.join("Roaming"),
            &root.join("Roaming"),
            &root.join("Local"),
            &root.join("Profile"),
            false,
        );
        let services = Services::with_paths(paths.clone()).expect("services");
        Fixture {
            _directory: directory,
            services,
            paths,
            secrets: Arc::new(FakeSecrets::default()),
        }
    }

    impl Fixture {
        fn registry(&self) -> Registry {
            Registry::new(&self.services, self.secrets.clone())
                .with_dialog(Arc::new(FakeDialog::empty()))
        }

        fn registry_with(&self, dialog: Arc<FakeDialog>, factory: Arc<FakeFactory>) -> Registry {
            Registry::new(&self.services, self.secrets.clone())
                .with_dialog(dialog)
                .with_factory(factory)
        }

        fn settings_text(&self) -> String {
            std::fs::read_to_string(&self.paths.settings_file).unwrap_or_default()
        }
    }

    fn imgur_input(id: &str, client_id: Option<&str>) -> DestinationInput {
        DestinationInput {
            id: Some(id.to_owned()),
            kind: DestinationKindDto::Imgur,
            name: None,
            client_id: client_id.map(str::to_owned),
            endpoint: None,
            region: None,
            bucket: None,
            access_key_id: None,
            secret_access_key: None,
            path_prefix: None,
            public_url_pattern: None,
            acl_public_read: None,
            method: None,
            request_url: None,
            body: None,
            file_form_field: None,
            link: None,
            thumbnail: None,
            deletion: None,
            error_message: None,
            headers: Vec::new(),
            imported_from_sxcu: None,
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            remote_folder: None,
            password: None,
            passphrase: None,
        }
    }

    fn custom_input(id: &str) -> DestinationInput {
        DestinationInput {
            id: Some(id.to_owned()),
            kind: DestinationKindDto::Custom,
            name: Some("My server".to_owned()),
            client_id: None,
            endpoint: None,
            region: None,
            bucket: None,
            access_key_id: None,
            secret_access_key: None,
            path_prefix: None,
            public_url_pattern: None,
            acl_public_read: None,
            method: Some("POST".to_owned()),
            request_url: Some("https://example.test/upload".to_owned()),
            body: Some("MultipartFormData".to_owned()),
            file_form_field: Some("file".to_owned()),
            link: Some("{json:$.data.url}".to_owned()),
            thumbnail: Some("{json:$.data.thumb}".to_owned()),
            deletion: None,
            error_message: None,
            headers: vec![DestinationHeaderInput {
                name: "Authorization".to_owned(),
                value: Some("Bearer top-secret".to_owned()),
            }],
            imported_from_sxcu: Some(false),
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            remote_folder: None,
            password: None,
            passphrase: None,
        }
    }

    fn sxcu_bytes() -> Vec<u8> {
        br#"{
            "Version": "13.7.0",
            "Name": "Example host",
            "DestinationType": "ImageUploader",
            "RequestMethod": "POST",
            "RequestURL": "https://example.test/upload",
            "Headers": { "Authorization": "Bearer top-secret" },
            "Body": "MultipartFormData",
            "FileFormName": "file",
            "URL": "{json:$.data.url}",
            "ThumbnailURL": "{json:$.data.thumb}",
            "DeletionURL": "{json:$.data.delete}",
            "ErrorMessage": "{json:$.error}"
        }"#
        .to_vec()
    }

    fn list(registry: &Registry) -> Vec<DestinationViewDto> {
        match crate::dto::upload::DestinationResp::Destinations(registry.list().expect("list")) {
            crate::dto::upload::DestinationResp::Destinations(items) => items,
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_fresh_install_has_no_destinations_and_uses_the_plan_defaults() {
        let fixture = fixture();
        let registry = fixture.registry();
        assert!(list(&registry).is_empty());
        let settings = fixture.services.settings_snapshot().expect("settings");
        assert_eq!(settings.uploads.images.as_str(), "save_locally_only");
        assert_eq!(
            settings.uploads.videos_and_gifs.as_str(),
            "save_locally_only"
        );
        assert_eq!(settings.uploads.text_and_code.as_str(), "save_locally_only");
        assert_eq!(settings.uploads.other_files.as_str(), "save_locally_only");
        assert_eq!(settings.uploads.link_shortener.as_str(), "none");
    }

    #[test]
    fn imgur_add_save_and_status_use_the_credential_store() {
        let fixture = fixture();
        let registry = fixture.registry();
        let added = registry.add(DestinationKindDto::Imgur).expect("add");
        assert_eq!(added.id, "imgur");
        assert_eq!(added.status, DestinationStatusDto::NotSetUp);
        assert_eq!(added.action, DestinationActionDto::Configure);
        assert!(!added.client_id_saved);

        let saved = registry
            .save(imgur_input("imgur", Some("a1b2c3d4")))
            .expect("save");
        assert_eq!(saved.status, DestinationStatusDto::Configured);
        assert!(saved.client_id_saved);
        assert!(fixture.secrets.has("imgur", CLIENT_ID_FIELD));
        assert!(!fixture.settings_text().contains("a1b2c3d4"));
        let json = serde_json::to_string(&saved).expect("json");
        assert!(!json.contains("a1b2c3d4"));
        assert!(!format!("{saved:?}").contains("a1b2c3d4"));
        let input = imgur_input("imgur", Some("a1b2c3d4"));
        assert!(!format!("{input:?}").contains("a1b2c3d4"));
        assert_eq!(saved.detail, "Image host · anonymous uploads");
    }

    #[test]
    fn imgur_requires_a_client_id_once_and_keeps_the_saved_one() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Imgur).expect("add");
        let error = registry
            .save(imgur_input("imgur", None))
            .expect_err("empty");
        assert_eq!(error.to_string(), "Enter the Imgur Client ID.");
        registry
            .save(imgur_input("imgur", Some("a1b2c3d4")))
            .expect("first save");
        let saved = registry
            .save(imgur_input("imgur", None))
            .expect("keep saved");
        assert!(saved.configured);
    }

    #[test]
    fn s3_fields_round_trip_and_secrets_stay_out_of_settings() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::S3).expect("add");
        let input = DestinationInput {
            id: Some("s3".to_owned()),
            kind: DestinationKindDto::S3,
            name: None,
            client_id: None,
            endpoint: Some("https://s3.example.test".to_owned()),
            region: Some("eu-west-2".to_owned()),
            bucket: Some("my-bucket".to_owned()),
            access_key_id: Some("AKIDEXAMPLE".to_owned()),
            secret_access_key: Some("top-secret-key".to_owned()),
            path_prefix: Some("shots".to_owned()),
            public_url_pattern: Some("https://cdn.example.test/<key>".to_owned()),
            acl_public_read: Some(true),
            method: None,
            request_url: None,
            body: None,
            file_form_field: None,
            link: None,
            thumbnail: None,
            deletion: None,
            error_message: None,
            headers: Vec::new(),
            imported_from_sxcu: None,
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            remote_folder: None,
            password: None,
            passphrase: None,
        };
        let saved = registry.save(input.clone()).expect("save");
        assert_eq!(saved.detail, "Bucket my-bucket · eu-west-2");
        assert_eq!(saved.status, DestinationStatusDto::Configured);
        assert_eq!(saved.action, DestinationActionDto::Configure);
        assert!(saved.endpoint.as_deref() == Some("https://s3.example.test"));
        assert_eq!(saved.path_prefix.as_deref(), Some("shots"));
        assert_eq!(
            saved.public_url_pattern.as_deref(),
            Some("https://cdn.example.test/<key>")
        );
        assert!(saved.acl_public_read);
        assert!(saved.access_key_id_saved);
        assert!(saved.secret_access_key_saved);
        assert!(fixture.secrets.has("s3", ACCESS_KEY_ID_FIELD));
        assert!(fixture.secrets.has("s3", SECRET_ACCESS_KEY_FIELD));
        let text = fixture.settings_text();
        assert!(!text.contains("AKIDEXAMPLE"));
        assert!(!text.contains("top-secret-key"));
        assert!(!serde_json::to_string(&saved)
            .expect("json")
            .contains("top-secret-key"));

        let reloaded = list(&registry);
        assert_eq!(reloaded.len(), 1);
        assert_eq!(reloaded[0].bucket.as_deref(), Some("my-bucket"));
        assert_eq!(reloaded[0].region.as_deref(), Some("eu-west-2"));
    }

    #[test]
    fn s3_requires_region_bucket_and_both_keys() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::S3).expect("add");
        let mut input = DestinationInput {
            id: Some("s3".to_owned()),
            kind: DestinationKindDto::S3,
            name: None,
            client_id: None,
            endpoint: None,
            region: Some(String::new()),
            bucket: Some(String::new()),
            access_key_id: None,
            secret_access_key: None,
            path_prefix: None,
            public_url_pattern: Some(DEFAULT_PUBLIC_URL_PATTERN.to_owned()),
            acl_public_read: Some(false),
            method: None,
            request_url: None,
            body: None,
            file_form_field: None,
            link: None,
            thumbnail: None,
            deletion: None,
            error_message: None,
            headers: Vec::new(),
            imported_from_sxcu: None,
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            remote_folder: None,
            password: None,
            passphrase: None,
        };
        assert_eq!(
            registry
                .save(input.clone())
                .expect_err("region")
                .to_string(),
            "Enter the S3 region."
        );
        input.region = Some("eu-west-2".to_owned());
        assert_eq!(
            registry
                .save(input.clone())
                .expect_err("bucket")
                .to_string(),
            "Enter the S3 bucket."
        );
        input.bucket = Some("my-bucket".to_owned());
        assert_eq!(
            registry
                .save(input.clone())
                .expect_err("access")
                .to_string(),
            "Enter the S3 access key ID."
        );
        input.access_key_id = Some("AKIDEXAMPLE".to_owned());
        assert_eq!(
            registry.save(input).expect_err("secret").to_string(),
            "Enter the S3 secret access key."
        );
    }

    #[test]
    fn custom_save_maps_every_field_and_hides_header_values() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Custom).expect("add");
        let saved = registry.save(custom_input("custom")).expect("save custom");
        assert_eq!(saved.name, "My server");
        assert_eq!(saved.action, DestinationActionDto::Edit);
        assert_eq!(saved.status, DestinationStatusDto::Configured);
        assert_eq!(saved.method.as_deref(), Some("POST"));
        assert_eq!(saved.body.as_deref(), Some("MultipartFormData"));
        assert_eq!(saved.file_form_field.as_deref(), Some("file"));
        assert_eq!(saved.link.as_deref(), Some("{json:$.data.url}"));
        assert_eq!(saved.thumbnail.as_deref(), Some("{json:$.data.thumb}"));
        assert_eq!(saved.headers.len(), 1);
        assert_eq!(saved.headers[0].name, "Authorization");
        assert!(saved.headers[0].saved);
        assert!(fixture.secrets.has("custom", "header_authorization"));
        let text = fixture.settings_text();
        assert!(!text.contains("top-secret"));
        assert!(!serde_json::to_string(&saved)
            .expect("json")
            .contains("top-secret"));
    }

    #[test]
    fn custom_save_rejects_an_invalid_link_template() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Custom).expect("add");
        let mut input = custom_input("custom");
        input.link = Some("{json:}".to_owned());
        assert!(matches!(
            registry.save(input).expect_err("invalid"),
            ServiceError::Invalid(_)
        ));
    }

    #[test]
    fn removing_a_destination_deletes_its_secrets() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Imgur).expect("add");
        registry
            .save(imgur_input("imgur", Some("a1b2c3d4")))
            .expect("save");
        assert!(fixture.secrets.has("imgur", CLIENT_ID_FIELD));
        registry.remove("imgur").expect("remove");
        assert!(!fixture.secrets.has("imgur", CLIENT_ID_FIELD));
        assert!(list(&registry).is_empty());
    }

    #[test]
    fn sxcu_import_stores_header_values_and_reports_warnings() {
        let fixture = fixture();
        let registry = fixture.registry_with(
            Arc::new(FakeDialog::with_file(sxcu_bytes())),
            Arc::new(FakeFactory {
                uploader: FakeUploader::new(),
            }),
        );
        let (view, warnings) = registry.import_sxcu().expect("import");
        let view = view.expect("destination");
        assert_eq!(view.name, "Example host");
        assert_eq!(view.detail, "Custom uploader · imported from .sxcu");
        assert!(view.imported_from_sxcu);
        assert_eq!(view.status, DestinationStatusDto::Configured);
        assert!(view.headers[0].saved);
        assert!(fixture.secrets.has(&view.id, "header_authorization"));
        assert!(warnings.is_empty());
        assert!(!fixture.settings_text().contains("top-secret"));
    }

    #[test]
    fn sxcu_import_rejects_files_larger_than_64_kib() {
        let fixture = fixture();
        let mut bytes = vec![b' '; sxcu::MAX_FILE_BYTES + 1];
        bytes[0] = b'{';
        bytes[1] = b'}';
        let registry = fixture.registry_with(
            Arc::new(FakeDialog::with_file(bytes)),
            Arc::new(FakeFactory {
                uploader: FakeUploader::new(),
            }),
        );
        let error = registry.import_sxcu().expect_err("too large");
        assert_eq!(error.to_string(), "The .sxcu file is larger than 64 KiB.");
        assert!(list(&registry).is_empty());
    }

    #[test]
    fn sxcu_import_rejects_an_unsupported_scheme() {
        let fixture = fixture();
        let bytes = br#"{"Name":"A","RequestURL":"file:///C:/secret.txt","Body":"Binary","URL":"{response}"}"#.to_vec();
        let registry = fixture.registry_with(
            Arc::new(FakeDialog::with_file(bytes)),
            Arc::new(FakeFactory {
                uploader: FakeUploader::new(),
            }),
        );
        let error = registry.import_sxcu().expect_err("scheme");
        assert!(error.to_string().contains("scheme"));
    }

    #[test]
    fn sxcu_export_blanks_header_values_unless_requested() {
        let fixture = fixture();
        let dialog = Arc::new(FakeDialog::empty());
        let registry = fixture.registry_with(
            dialog.clone(),
            Arc::new(FakeFactory {
                uploader: FakeUploader::new(),
            }),
        );
        registry.add(DestinationKindDto::Custom).expect("add");
        registry.save(custom_input("custom")).expect("save");
        registry.export_sxcu("custom", false).expect("export");
        registry.export_sxcu("custom", true).expect("export");
        let saved = dialog.saved.lock().expect("dialog").clone();
        assert_eq!(saved.len(), 2);
        assert!(saved[0].0.ends_with(".sxcu"));
        assert!(!saved[0].1.contains("top-secret"));
        assert!(saved[1].1.contains("top-secret"));
        assert!(!fixture.settings_text().contains("top-secret"));
    }

    #[test]
    fn test_upload_goes_through_the_queue_with_a_fake_uploader() {
        let fixture = fixture();
        let uploader = FakeUploader::new();
        let factory = Arc::new(FakeFactory {
            uploader: uploader.clone(),
        });
        let registry = fixture.registry_with(Arc::new(FakeDialog::empty()), factory);
        registry.add(DestinationKindDto::Imgur).expect("add");
        registry
            .save(imgur_input("imgur", Some("a1b2c3d4")))
            .expect("save");
        let thread = UploaderThread::start(
            &fixture.services,
            fixture.secrets.clone(),
            snapline_upload::queue::RetryPolicy::new(0),
            Arc::new(NoFiles),
            Arc::new(registry),
        )
        .expect("thread");
        let service = thread.service();
        let response = service
            .handle(UploadReq::Destination(DestinationReq::Test {
                id: "imgur".to_owned(),
            }))
            .expect("test");
        let outcome = match response {
            UploadResp::Destination(crate::dto::upload::DestinationResp::Test { outcome }) => {
                outcome
            }
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(outcome.status, DestinationTestStatusDto::Succeeded);
        assert_eq!(
            outcome.link.as_deref(),
            Some("https://example.test/snapline-test.png")
        );
        assert_eq!(uploader.calls.load(Ordering::SeqCst), 1);
        thread.join().expect("join");
    }

    #[test]
    fn the_registry_resolves_both_the_id_and_the_display_name() {
        let fixture = fixture();
        let uploader = FakeUploader::new();
        let factory = Arc::new(FakeFactory {
            uploader: uploader.clone(),
        });
        let registry = fixture.registry_with(Arc::new(FakeDialog::empty()), factory);
        registry.add(DestinationKindDto::S3).expect("add");
        registry
            .save(DestinationInput {
                id: Some("s3".to_owned()),
                kind: DestinationKindDto::S3,
                name: None,
                client_id: None,
                endpoint: None,
                region: Some("eu-west-2".to_owned()),
                bucket: Some("my-bucket".to_owned()),
                access_key_id: Some("AKIDEXAMPLE".to_owned()),
                secret_access_key: Some("top-secret-key".to_owned()),
                path_prefix: None,
                public_url_pattern: Some(DEFAULT_PUBLIC_URL_PATTERN.to_owned()),
                acl_public_read: Some(false),
                method: None,
                request_url: None,
                body: None,
                file_form_field: None,
                link: None,
                thumbnail: None,
                deletion: None,
                error_message: None,
                headers: Vec::new(),
                imported_from_sxcu: None,
                host: None,
                port: None,
                username: None,
                auth_method: None,
                private_key_path: None,
                remote_folder: None,
                password: None,
                passphrase: None,
            })
            .expect("save");
        assert!(registry.uploader("s3").is_some());
        assert!(registry.uploader("Amazon S3").is_some());
        assert!(registry.uploader("missing").is_none());
    }

    #[test]
    fn form_mappings_match_the_sharex_labels() {
        assert_eq!(method_value("post").expect("post"), Method::Post);
        assert_eq!(method_value("PUT").expect("put"), Method::Put);
        assert_eq!(method_value("PATCH").expect("patch"), Method::Patch);
        assert!(method_value("DELETE").is_err());
        assert_eq!(
            body_value("MultipartFormData").expect("multipart"),
            sxcu::BodyType::MultipartFormData
        );
        assert_eq!(
            body_value("Binary").expect("binary"),
            sxcu::BodyType::Binary
        );
        assert_eq!(
            body_value("FormURLEncoded").expect("form"),
            sxcu::BodyType::FormUrlEncoded
        );
        assert_eq!(body_value("JSON").expect("json"), sxcu::BodyType::Json);
        assert!(body_value("XML").is_err());
        assert_eq!(safe_file_name("My server"), "My server");
        assert_eq!(safe_file_name("a/b:c"), "a_b_c");
        assert_eq!(test_file().len as usize, TEST_PNG.len());
    }

    fn gist_input(id: &str, token: Option<&str>, public: bool) -> DestinationInput {
        DestinationInput {
            id: Some(id.to_owned()),
            kind: DestinationKindDto::Gist,
            name: None,
            client_id: token.map(str::to_owned),
            endpoint: None,
            region: None,
            bucket: None,
            access_key_id: None,
            secret_access_key: None,
            path_prefix: None,
            public_url_pattern: None,
            acl_public_read: Some(public),
            method: None,
            request_url: None,
            body: None,
            file_form_field: None,
            link: None,
            thumbnail: None,
            deletion: None,
            error_message: None,
            headers: Vec::new(),
            imported_from_sxcu: None,
            host: None,
            port: None,
            username: None,
            auth_method: None,
            private_key_path: None,
            remote_folder: None,
            password: None,
            passphrase: None,
        }
    }

    fn sftp_input(id: &str) -> DestinationInput {
        DestinationInput {
            id: Some(id.to_owned()),
            kind: DestinationKindDto::Sftp,
            name: None,
            client_id: None,
            endpoint: None,
            region: None,
            bucket: None,
            access_key_id: None,
            secret_access_key: None,
            path_prefix: None,
            public_url_pattern: Some("https://cdn.example.test/shots/<key>".to_owned()),
            acl_public_read: None,
            method: None,
            request_url: None,
            body: None,
            file_form_field: None,
            link: None,
            thumbnail: None,
            deletion: None,
            error_message: None,
            headers: Vec::new(),
            imported_from_sxcu: None,
            host: Some("sftp.example.test".to_owned()),
            port: Some(2222),
            username: Some("aaron".to_owned()),
            auth_method: Some("password".to_owned()),
            private_key_path: None,
            remote_folder: Some("/srv/shots".to_owned()),
            password: Some("correct horse".to_owned()),
            passphrase: None,
        }
    }

    struct FakeHostKeys;

    impl HostKeyStore for FakeHostKeys {
        fn state(
            &self,
            _host: &str,
            _port: u16,
            _key: &[u8],
        ) -> Result<HostKeyState, snapline_upload::error::UploadError> {
            Ok(HostKeyState::Unknown)
        }

        fn remember(
            &self,
            _host: &str,
            _port: u16,
            _algorithm: &str,
            _key: &[u8],
        ) -> Result<(), snapline_upload::error::UploadError> {
            Ok(())
        }
    }

    struct FakeConfirmer;

    impl HostKeyConfirmer for FakeConfirmer {
        fn confirm(&self, _prompt: &HostKeyPrompt) -> bool {
            true
        }
    }

    #[test]
    fn destination_kinds_round_trip_through_json() {
        for kind in [
            DestinationKindDto::Imgur,
            DestinationKindDto::S3,
            DestinationKindDto::Gist,
            DestinationKindDto::Sftp,
            DestinationKindDto::Custom,
        ] {
            let json = serde_json::to_string(&kind).expect("kind json");
            let back: DestinationKindDto = serde_json::from_str(&json).expect("kind round trip");
            assert_eq!(back, kind);
        }
        assert_eq!(
            serde_json::to_string(&DestinationKindDto::Gist).expect("gist"),
            "\"gist\""
        );
        assert_eq!(
            serde_json::to_string(&DestinationKindDto::Sftp).expect("sftp"),
            "\"sftp\""
        );
    }

    #[test]
    fn destination_configs_round_trip_through_json() {
        let gist = DestinationConfig::Gist {
            id: "gist".to_owned(),
            public: true,
        };
        let json = serde_json::to_string(&gist).expect("gist config");
        let back: DestinationConfig = serde_json::from_str(&json).expect("gist back");
        assert_eq!(back, gist);

        let sftp = DestinationConfig::Sftp {
            id: "sftp".to_owned(),
            host: "sftp.example.test".to_owned(),
            port: 2222,
            username: "aaron".to_owned(),
            auth_method: "private_key".to_owned(),
            private_key_path: Some("C:\\keys\\id_ed25519".to_owned()),
            remote_folder: "/srv/shots".to_owned(),
            public_url_pattern: "https://cdn.example.test/<key>".to_owned(),
        };
        let json = serde_json::to_string(&sftp).expect("sftp config");
        let back: DestinationConfig = serde_json::from_str(&json).expect("sftp back");
        assert_eq!(back, sftp);
    }

    #[test]
    fn gist_add_save_status_and_secret_isolation() {
        let fixture = fixture();
        let registry = fixture.registry();
        let added = registry.add(DestinationKindDto::Gist).expect("add");
        assert_eq!(added.id, "gist");
        assert_eq!(added.kind, DestinationKindDto::Gist);
        assert_eq!(added.name, "GitHub Gist");
        assert_eq!(added.detail, "Text and code snippets");
        assert_eq!(added.status, DestinationStatusDto::NotSetUp);
        assert_eq!(added.action, DestinationActionDto::SetUp);
        assert!(!added.client_id_saved);

        let saved = registry
            .save(gist_input("gist", Some("ghp_example_token"), true))
            .expect("save");
        assert_eq!(saved.status, DestinationStatusDto::Configured);
        assert_eq!(saved.action, DestinationActionDto::Configure);
        assert!(saved.client_id_saved);
        assert!(saved.acl_public_read);
        assert_eq!(saved.detail, "Text and code snippets");
        assert!(fixture.secrets.has("gist", GIST_TOKEN_FIELD));
        assert!(!fixture.secrets.has("gist", SFTP_PASSWORD_FIELD));
        let text = fixture.settings_text();
        assert!(!text.contains("ghp_example_token"));
        let json = serde_json::to_string(&saved).expect("json");
        assert!(!json.contains("ghp_example_token"));
        assert!(!format!("{saved:?}").contains("ghp_example_token"));
        let input = gist_input("gist", Some("ghp_example_token"), true);
        assert!(!format!("{input:?}").contains("ghp_example_token"));

        let kept = registry
            .save(gist_input("gist", None, false))
            .expect("keep saved token");
        assert!(kept.configured);
        assert!(kept.client_id_saved);
        assert!(!kept.acl_public_read);
        assert!(fixture.secrets.has("gist", GIST_TOKEN_FIELD));
    }

    #[test]
    fn gist_requires_a_token_once() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Gist).expect("add");
        let error = registry
            .save(gist_input("gist", None, false))
            .expect_err("missing token");
        assert_eq!(error.to_string(), "Enter the GitHub token.");
        let error = registry
            .save(gist_input("gist", Some("bad token\n"), false))
            .expect_err("invalid token");
        assert_eq!(error.to_string(), "The GitHub token is not valid.");
    }

    #[test]
    fn sftp_password_fields_round_trip_and_secrets_stay_out_of_settings() {
        let fixture = fixture();
        let registry = fixture.registry();
        let added = registry.add(DestinationKindDto::Sftp).expect("add");
        assert_eq!(added.id, "sftp");
        assert_eq!(added.kind, DestinationKindDto::Sftp);
        assert_eq!(added.name, "SFTP");
        assert_eq!(added.detail, "Your own server over SSH");
        assert_eq!(added.port, Some(22));
        assert_eq!(added.host.as_deref(), Some(""));
        assert_eq!(added.status, DestinationStatusDto::NotSetUp);
        assert_eq!(added.action, DestinationActionDto::SetUp);

        let saved = registry.save(sftp_input("sftp")).expect("save");
        assert_eq!(saved.status, DestinationStatusDto::Configured);
        assert_eq!(saved.action, DestinationActionDto::Configure);
        assert_eq!(saved.host.as_deref(), Some("sftp.example.test"));
        assert_eq!(saved.port, Some(2222));
        assert_eq!(saved.username.as_deref(), Some("aaron"));
        assert_eq!(saved.auth_method.as_deref(), Some("password"));
        assert_eq!(saved.private_key_path, None);
        assert_eq!(saved.remote_folder.as_deref(), Some("/srv/shots"));
        assert_eq!(
            saved.public_url_pattern.as_deref(),
            Some("https://cdn.example.test/shots/<key>")
        );
        assert!(saved.password_saved);
        assert!(!saved.passphrase_saved);
        assert!(fixture.secrets.has("sftp", SFTP_PASSWORD_FIELD));
        assert!(!fixture.secrets.has("sftp", SFTP_PASSPHRASE_FIELD));
        let text = fixture.settings_text();
        assert!(!text.contains("correct horse"));
        assert!(!serde_json::to_string(&saved)
            .expect("json")
            .contains("correct horse"));
        assert!(!format!("{saved:?}").contains("correct horse"));
    }

    #[test]
    fn sftp_private_key_auth_and_method_changes_delete_the_other_secret() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Sftp).expect("add");
        registry.save(sftp_input("sftp")).expect("password save");
        assert!(fixture.secrets.has("sftp", SFTP_PASSWORD_FIELD));

        let mut key = sftp_input("sftp");
        key.auth_method = Some("private_key".to_owned());
        key.private_key_path = Some("C:\\keys\\id_ed25519".to_owned());
        key.password = None;
        key.passphrase = Some("key secret".to_owned());
        let saved = registry.save(key).expect("key save");
        assert_eq!(saved.auth_method.as_deref(), Some("private_key"));
        assert_eq!(
            saved.private_key_path.as_deref(),
            Some("C:\\keys\\id_ed25519")
        );
        assert!(saved.passphrase_saved);
        assert!(!saved.password_saved);
        assert!(!fixture.secrets.has("sftp", SFTP_PASSWORD_FIELD));
        assert!(fixture.secrets.has("sftp", SFTP_PASSPHRASE_FIELD));
        assert!(!fixture.settings_text().contains("key secret"));

        let mut password = sftp_input("sftp");
        password.auth_method = Some("password".to_owned());
        password.password = None;
        let error = registry.save(password).expect_err("password required");
        assert_eq!(error.to_string(), "Enter the SFTP password.");
        assert!(fixture.secrets.has("sftp", SFTP_PASSPHRASE_FIELD));

        let mut password = sftp_input("sftp");
        password.auth_method = Some("password".to_owned());
        let saved = registry.save(password).expect("password again");
        assert_eq!(saved.auth_method.as_deref(), Some("password"));
        assert_eq!(saved.private_key_path, None);
        assert!(saved.password_saved);
        assert!(!saved.passphrase_saved);
        assert!(fixture.secrets.has("sftp", SFTP_PASSWORD_FIELD));
        assert!(!fixture.secrets.has("sftp", SFTP_PASSPHRASE_FIELD));
    }

    #[test]
    fn sftp_validates_the_required_fields() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Sftp).expect("add");

        let mut input = sftp_input("sftp");
        input.host = Some("  ".to_owned());
        assert_eq!(
            registry.save(input.clone()).expect_err("host").to_string(),
            "Enter the SFTP host."
        );
        input.host = Some("sftp.example.test".to_owned());
        input.port = Some(0);
        assert_eq!(
            registry.save(input.clone()).expect_err("port").to_string(),
            "The SFTP port must be between 1 and 65535."
        );
        input.port = Some(22);
        input.username = Some("".to_owned());
        assert_eq!(
            registry
                .save(input.clone())
                .expect_err("username")
                .to_string(),
            "Enter the SFTP username."
        );
        input.username = Some("aaron".to_owned());
        input.public_url_pattern = Some("https://example.test/no-token".to_owned());
        assert_eq!(
            registry
                .save(input.clone())
                .expect_err("pattern")
                .to_string(),
            "The public URL pattern must contain <key>."
        );
        input.public_url_pattern = Some("https://example.test/<key>".to_owned());
        input.auth_method = Some("private_key".to_owned());
        input.private_key_path = None;
        assert_eq!(
            registry.save(input.clone()).expect_err("key").to_string(),
            "Choose the SFTP private key file."
        );
        input.auth_method = Some("carrier_pigeon".to_owned());
        assert_eq!(
            registry.save(input).expect_err("auth").to_string(),
            "The SFTP authentication method is not valid."
        );
    }

    #[test]
    fn removing_gist_and_sftp_deletes_every_secret() {
        let fixture = fixture();
        let registry = fixture.registry();
        registry.add(DestinationKindDto::Gist).expect("add gist");
        registry.add(DestinationKindDto::Sftp).expect("add sftp");
        registry
            .save(gist_input("gist", Some("ghp_example_token"), false))
            .expect("save gist");
        let mut key = sftp_input("sftp");
        key.auth_method = Some("private_key".to_owned());
        key.private_key_path = Some("C:\\keys\\id_ed25519".to_owned());
        key.password = None;
        key.passphrase = Some("key secret".to_owned());
        registry.save(key).expect("save sftp");

        registry.remove("gist").expect("remove gist");
        assert!(!fixture.secrets.has("gist", GIST_TOKEN_FIELD));
        assert!(fixture.secrets.has("sftp", SFTP_PASSPHRASE_FIELD));
        assert_eq!(list(&registry).len(), 1);

        registry.remove("sftp").expect("remove sftp");
        assert!(!fixture.secrets.has("sftp", SFTP_PASSPHRASE_FIELD));
        assert!(!fixture.secrets.has("sftp", SFTP_PASSWORD_FIELD));
        assert!(list(&registry).is_empty());
    }

    #[test]
    fn gist_and_sftp_test_uploads_go_through_the_queue() {
        let uploader = FakeUploader::new();
        let factory = Arc::new(FakeFactory {
            uploader: uploader.clone(),
        });
        let fixture = fixture();
        let registry = fixture.registry_with(Arc::new(FakeDialog::empty()), factory);
        registry.add(DestinationKindDto::Gist).expect("add gist");
        registry
            .save(gist_input("gist", Some("ghp_example_token"), false))
            .expect("save gist");
        registry.add(DestinationKindDto::Sftp).expect("add sftp");
        registry.save(sftp_input("sftp")).expect("save sftp");
        let thread = UploaderThread::start(
            &fixture.services,
            fixture.secrets.clone(),
            snapline_upload::queue::RetryPolicy::new(0),
            Arc::new(NoFiles),
            Arc::new(registry),
        )
        .expect("thread");
        let service = thread.service();
        for (id, file_name) in [("gist", TEST_GIST_FILE_NAME), ("sftp", TEST_FILE_NAME)] {
            let response = service
                .handle(UploadReq::Destination(DestinationReq::Test {
                    id: id.to_owned(),
                }))
                .expect("test upload");
            let outcome = match response {
                UploadResp::Destination(crate::dto::upload::DestinationResp::Test { outcome }) => {
                    outcome
                }
                other => panic!("unexpected response: {other:?}"),
            };
            assert_eq!(outcome.status, DestinationTestStatusDto::Succeeded, "{id}");
            let expected = format!("https://example.test/{file_name}");
            assert_eq!(outcome.link.as_deref(), Some(expected.as_str()), "{id}");
        }
        assert_eq!(uploader.calls.load(Ordering::SeqCst), 2);
        thread.join().expect("join");
    }

    #[test]
    fn the_registry_injects_sftp_host_keys_and_confirmation() {
        let fixture = fixture();
        let host_keys: Arc<dyn HostKeyStore> = Arc::new(FakeHostKeys);
        let confirmer: Arc<dyn HostKeyConfirmer> = Arc::new(FakeConfirmer);
        let registry = fixture
            .registry()
            .with_sftp(host_keys.clone(), confirmer.clone());
        assert!(Arc::ptr_eq(&registry.host_keys(), &host_keys));
        assert!(Arc::ptr_eq(&registry.host_key_confirmer(), &confirmer));

        assert!(matches!(
            UnavailableHostKeyStore.state("example.com", 22, b"key"),
            Err(snapline_upload::error::UploadError::Permanent(_))
        ));
        assert!(matches!(
            UnavailableHostKeyStore.remember("example.com", 22, "ssh-ed25519", b"key"),
            Err(snapline_upload::error::UploadError::Permanent(_))
        ));
        assert!(!UnavailableHostKeyConfirmer.confirm(&HostKeyPrompt {
            host: "example.com".to_owned(),
            port: 22,
            algorithm: "ssh-ed25519".to_owned(),
            fingerprint: "SHA256:test".to_owned(),
        }));
    }
}
