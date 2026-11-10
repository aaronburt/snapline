use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Proxy};

use crate::error::UploadError;
use crate::http::{
    HttpClient, HttpTimeout, HttpTransport, Multipart, ProxySetting, Request, USER_AGENT,
};
use crate::queue::{
    DeletionLink, SecretStore, UploadFile, Uploaded, Uploader, MAX_BUFFERED_UPLOAD_BYTES,
};

pub const DEFAULT_DEST_ID: &str = "imgur";
pub const CLIENT_ID_FIELD: &str = "client_id";
pub const API_BASE: &str = "https://api.imgur.com/3";
pub const DELETION_BASE: &str = "https://imgur.com/delete";
pub const SECRET_TARGET_PREFIX: &str = "SnapLine:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteResponse {
    pub status: u16,
    pub retry_after: Option<Duration>,
}

pub trait DeleteTransport: Send + Sync {
    fn delete(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<DeleteResponse, UploadError>;
}

pub struct UreqDeleter {
    agent: Agent,
}

impl UreqDeleter {
    pub fn new(timeout: HttpTimeout, proxy: ProxySetting) -> Result<Self, UploadError> {
        let proxy = match proxy {
            ProxySetting::System => Proxy::try_from_env(),
            ProxySetting::None => None,
            ProxySetting::Custom(url) => Some(Proxy::new(&url).map_err(|_| {
                UploadError::Permanent("The proxy address is not valid.".to_owned())
            })?),
        };
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_global(Some(Duration::from_secs(timeout.seconds())))
            .tls_config(tls)
            .proxy(proxy)
            .build();
        Ok(Self {
            agent: Agent::new_with_config(config),
        })
    }
}

impl DeleteTransport for UreqDeleter {
    fn delete(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<DeleteResponse, UploadError> {
        let mut builder = ureq::http::Request::builder()
            .method(ureq::http::Method::DELETE)
            .uri(url);
        for (name, value) in headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let request = builder
            .body(Vec::new())
            .map_err(|_| UploadError::Permanent("The request could not be built.".to_owned()))?;
        let response = self
            .agent
            .run(request)
            .map_err(|error| UploadError::from_ureq(&error))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        Ok(DeleteResponse {
            status,
            retry_after,
        })
    }
}

pub struct ImgurUploader {
    dest_id: String,
    api_base: String,
    http: Arc<dyn HttpTransport>,
    deleter: Arc<dyn DeleteTransport>,
}

impl ImgurUploader {
    pub fn new(
        dest_id: impl Into<String>,
        http: Arc<dyn HttpTransport>,
        deleter: Arc<dyn DeleteTransport>,
    ) -> Self {
        Self {
            dest_id: dest_id.into(),
            api_base: API_BASE.to_owned(),
            http,
            deleter,
        }
    }

    pub fn from_settings(
        dest_id: impl Into<String>,
        timeout: HttpTimeout,
        proxy: ProxySetting,
    ) -> Result<Self, UploadError> {
        let http = HttpClient::new(timeout, proxy.clone())?;
        let deleter = UreqDeleter::new(timeout, proxy)?;
        Ok(Self::new(dest_id, Arc::new(http), Arc::new(deleter)))
    }

    pub fn imgur(http: Arc<dyn HttpTransport>, deleter: Arc<dyn DeleteTransport>) -> Self {
        Self::new(DEFAULT_DEST_ID, http, deleter)
    }

    pub fn dest_id(&self) -> &str {
        &self.dest_id
    }

    pub fn client_id_key(&self) -> String {
        format!("{SECRET_TARGET_PREFIX}{}:{CLIENT_ID_FIELD}", self.dest_id)
    }

    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into().trim_end_matches('/').to_owned();
        self
    }

    pub fn upload_with_metadata(
        &self,
        file: &UploadFile,
        title: Option<&str>,
        description: Option<&str>,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        let bytes = file.read_all(MAX_BUFFERED_UPLOAD_BYTES)?;
        let client_id = self.client_id(secrets)?;
        let mut form = Multipart::new().file(
            "image",
            file.file_name.clone(),
            file.content_type.clone(),
            bytes,
        );
        if let Some(title) = title.filter(|value| !value.is_empty()) {
            form = form.text("title", title);
        }
        if let Some(description) = description.filter(|value| !value.is_empty()) {
            form = form.text("description", description);
        }
        let request = Request::post(format!("{}/image", self.api_base))
            .header("Authorization", format!("Client-ID {client_id}"))
            .multipart(form);
        let response = self.http.send(request)?;
        let envelope: Envelope = response.json()?;
        if !envelope.success {
            return Err(declared_status_error(envelope.status));
        }
        let data = envelope.data.ok_or_else(|| {
            UploadError::Permanent("Imgur did not return image details.".to_owned())
        })?;
        let ImgurData {
            link,
            deletehash,
            id: _image_id,
        } = data;
        let link = link
            .filter(|value| !value.is_empty())
            .ok_or_else(|| UploadError::Permanent("Imgur did not return a link.".to_owned()))?;
        let deletion_link = deletehash.filter(|value| !value.is_empty()).map(|hash| {
            DeletionLink::new(self.dest_id.clone(), format!("{DELETION_BASE}/{hash}"))
                .with_token(hash)
        });
        Ok(Uploaded {
            link,
            page_link: None,
            deletion_link,
        })
    }

    fn client_id(&self, secrets: &dyn SecretStore) -> Result<String, UploadError> {
        match secrets.get(&self.dest_id, CLIENT_ID_FIELD) {
            Ok(Some(mut bytes)) => {
                let value = std::str::from_utf8(&bytes)
                    .ok()
                    .filter(|value| {
                        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic())
                    })
                    .map(str::to_owned);
                bytes.fill(0);
                value.ok_or_else(|| {
                    UploadError::Permanent("The saved Imgur Client ID is not valid.".to_owned())
                })
            }
            Ok(None) => Err(UploadError::Permanent(
                "No Imgur Client ID is saved. Add one in Settings.".to_owned(),
            )),
            Err(_) => Err(UploadError::Permanent(
                "The saved Imgur Client ID could not be read.".to_owned(),
            )),
        }
    }
}

impl Uploader for ImgurUploader {
    fn upload(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        self.upload_with_metadata(file, None, None, secrets)
    }

    fn delete(&self, link: &DeletionLink, secrets: &dyn SecretStore) -> Result<(), UploadError> {
        let client_id = self.client_id(secrets)?;
        let hash = deletion_hash(link)?;
        let headers = [(
            String::from("Authorization"),
            format!("Client-ID {client_id}"),
        )];
        let url = format!("{}/image/{hash}", self.api_base);
        let response = self.deleter.delete(&url, &headers)?;
        if response.status == 404 || (200..=299).contains(&response.status) {
            return Ok(());
        }
        Err(UploadError::from_status_with_retry_after(
            response.status,
            response.retry_after,
        ))
    }
}

#[derive(Debug, Deserialize)]
struct Envelope {
    success: bool,
    status: u16,
    data: Option<ImgurData>,
}

#[derive(Debug, Deserialize)]
struct ImgurData {
    link: Option<String>,
    deletehash: Option<String>,
    id: Option<String>,
}

fn declared_status_error(status: u16) -> UploadError {
    if status == 0 || (200..=299).contains(&status) {
        UploadError::Permanent("Imgur rejected the upload.".to_owned())
    } else {
        UploadError::from_status(status)
    }
}

fn deletion_hash(link: &DeletionLink) -> Result<String, UploadError> {
    if let Some(token) = link
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        return valid_hash(token);
    }
    let trimmed = link.url.trim();
    if trimmed.is_empty() {
        return Err(UploadError::Permanent(
            "The Imgur deletion link is not valid.".to_owned(),
        ));
    }
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let hash = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    valid_hash(hash)
}

fn valid_hash(hash: &str) -> Result<String, UploadError> {
    if hash.is_empty()
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(UploadError::Permanent(
            "The Imgur deletion link is not valid.".to_owned(),
        ));
    }
    Ok(hash.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};
    use std::thread::JoinHandle;

    use tiny_http::{Request as StubRequest, Response as StubResponse, Server};

    use super::*;
    use crate::http::Method;
    use crate::queue::SecretError;

    const CLIENT_ID: &str = "a1b2c3d4e5f6";

    struct StubSecrets {
        value: Option<Vec<u8>>,
        error: Option<SecretError>,
        requested: Mutex<Vec<String>>,
    }

    impl StubSecrets {
        fn with_client_id(value: &str) -> Self {
            Self {
                value: Some(value.as_bytes().to_vec()),
                error: None,
                requested: Mutex::new(Vec::new()),
            }
        }

        fn missing() -> Self {
            Self {
                value: None,
                error: None,
                requested: Mutex::new(Vec::new()),
            }
        }

        fn failing(error: SecretError) -> Self {
            Self {
                value: None,
                error: Some(error),
                requested: Mutex::new(Vec::new()),
            }
        }

        fn requested(&self) -> MutexGuard<'_, Vec<String>> {
            self.requested.lock().expect("secret requests")
        }
    }

    impl SecretStore for StubSecrets {
        fn get(&self, dest_id: &str, field: &str) -> Result<Option<Vec<u8>>, SecretError> {
            self.requested
                .lock()
                .expect("secret requests")
                .push(format!("{dest_id}:{field}"));
            match &self.error {
                Some(error) => Err(*error),
                None => Ok(self.value.clone()),
            }
        }
    }

    struct RecordingHttp {
        requests: Mutex<Vec<Request>>,
        error: Option<UploadError>,
    }

    impl RecordingHttp {
        fn failing(error: UploadError) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                error: Some(error),
            }
        }

        fn take(&self) -> Vec<Request> {
            std::mem::take(&mut *self.requests.lock().expect("requests"))
        }
    }

    impl HttpTransport for RecordingHttp {
        fn execute(&self, request: Request) -> Result<crate::http::Response, UploadError> {
            self.requests.lock().expect("requests").push(request);
            Err(self
                .error
                .clone()
                .unwrap_or_else(|| UploadError::Permanent("recorded".to_owned())))
        }
    }

    struct RecordingDeleter {
        response: Result<DeleteResponse, UploadError>,
        calls: Mutex<Vec<(String, Vec<(String, String)>)>>,
    }

    impl RecordingDeleter {
        fn returning(status: u16, retry_after: Option<Duration>) -> Self {
            Self {
                response: Ok(DeleteResponse {
                    status,
                    retry_after,
                }),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> MutexGuard<'_, Vec<(String, Vec<(String, String)>)>> {
            self.calls.lock().expect("delete calls")
        }
    }

    impl DeleteTransport for RecordingDeleter {
        fn delete(
            &self,
            url: &str,
            headers: &[(String, String)],
        ) -> Result<DeleteResponse, UploadError> {
            self.calls
                .lock()
                .expect("delete calls")
                .push((url.to_owned(), headers.to_vec()));
            self.response.clone()
        }
    }

    struct Captured {
        method: String,
        authorization: String,
        content_type: String,
        body: Vec<u8>,
    }

    fn serve(status: u16, body: &'static str) -> (String, JoinHandle<Captured>) {
        let server = Server::http("127.0.0.1:0").expect("stub server");
        let base = format!("http://{}", server.server_addr());
        let handle = std::thread::spawn(move || {
            let mut request = server.recv().expect("stub request");
            let captured = Captured {
                method: request.method().to_string(),
                authorization: header(&request, "Authorization"),
                content_type: header(&request, "Content-Type"),
                body: {
                    let mut body = Vec::new();
                    let _ = request.as_reader().read_to_end(&mut body);
                    body
                },
            };
            let _ = request.respond(StubResponse::from_string(body).with_status_code(status));
            captured
        });
        (base, handle)
    }

    fn header(request: &StubRequest, name: &'static str) -> String {
        request
            .headers()
            .iter()
            .find(|header| header.field.equiv(name))
            .map(|header| header.value.as_str().to_owned())
            .unwrap_or_default()
    }

    fn test_client() -> Arc<dyn HttpTransport> {
        Arc::new(
            HttpClient::new(HttpTimeout::ThirtySeconds, ProxySetting::None).expect("http client"),
        )
    }

    fn uploader(base: &str, deleter: RecordingDeleter) -> ImgurUploader {
        ImgurUploader::new(DEFAULT_DEST_ID, test_client(), Arc::new(deleter))
            .with_api_base(format!("{base}/3"))
    }

    fn file() -> UploadFile {
        UploadFile::new(
            PathBuf::from("captures/shot.png"),
            "shot.png",
            "image/png",
            vec![1_u8, 2, 3],
        )
    }

    fn upload_status(status: u16, body: &'static str) -> UploadError {
        let (base, handle) = serve(status, body);
        let uploader = uploader(&base, RecordingDeleter::returning(200, None));
        let secrets = StubSecrets::with_client_id(CLIENT_ID);
        let error = uploader
            .upload(&file(), &secrets)
            .expect_err("upload error");
        handle.join().expect("stub server");
        error
    }

    #[test]
    fn upload_sends_the_documented_request_and_parses_the_response() {
        let (base, handle) = serve(
            200,
            r#"{"data":{"id":"abc123","deletehash":"dh123","link":"https://i.imgur.com/abc123.png"},"success":true,"status":200}"#,
        );
        let uploader = uploader(&base, RecordingDeleter::returning(200, None));
        let secrets = StubSecrets::with_client_id(CLIENT_ID);
        let uploaded = uploader
            .upload_with_metadata(&file(), Some("A title"), Some("A description"), &secrets)
            .expect("upload");
        let captured = handle.join().expect("stub server");
        assert_eq!(captured.method, "POST");
        assert_eq!(captured.authorization, format!("Client-ID {CLIENT_ID}"));
        assert!(captured
            .content_type
            .starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8_lossy(&captured.body);
        assert!(body.contains("name=\"image\"; filename=\"shot.png\""));
        assert!(body.contains("Content-Type: image/png"));
        assert!(body.contains("name=\"title\"\r\n\r\nA title"));
        assert!(body.contains("name=\"description\"\r\n\r\nA description"));
        assert_eq!(uploaded.link, "https://i.imgur.com/abc123.png");
        assert_eq!(uploaded.page_link, None);
        assert_eq!(
            uploaded.deletion_link,
            Some(
                DeletionLink::new(DEFAULT_DEST_ID, "https://imgur.com/delete/dh123")
                    .with_token("dh123")
            )
        );
        assert_eq!(
            secrets.requested().as_slice(),
            ["imgur:client_id".to_owned()]
        );
    }

    #[test]
    fn upload_reads_the_client_id_from_the_documented_key() {
        let http = Arc::new(RecordingHttp::failing(UploadError::Permanent(
            "recorded".to_owned(),
        )));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            http.clone(),
            Arc::new(RecordingDeleter::returning(200, None)),
        );
        let secrets = StubSecrets::with_client_id(CLIENT_ID);
        let _ = uploader.upload_with_metadata(
            &file(),
            Some("A title"),
            Some("A description"),
            &secrets,
        );
        assert_eq!(
            secrets.requested().as_slice(),
            ["imgur:client_id".to_owned()]
        );
        assert_eq!(uploader.client_id_key(), "SnapLine:imgur:client_id");
        let requests = http.take();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), Method::Post);
        assert_eq!(requests[0].url(), "https://api.imgur.com/3/image");
        assert_eq!(
            requests[0].headers(),
            &[
                ("Authorization".to_owned(), format!("Client-ID {CLIENT_ID}")),
                (
                    "Content-Type".to_owned(),
                    "multipart/form-data; boundary=----SnaplineFormBoundaryJ7x2Vq9mR4tL".to_owned()
                ),
            ]
        );
        let body = String::from_utf8_lossy(requests[0].body_bytes());
        assert!(body.contains("name=\"image\"; filename=\"shot.png\""));
        assert!(body.contains("name=\"title\"\r\n\r\nA title"));
        assert!(body.contains("name=\"description\"\r\n\r\nA description"));
    }

    #[test]
    fn a_missing_client_id_is_permanent_and_sends_nothing() {
        let http = Arc::new(RecordingHttp::failing(UploadError::Permanent(
            "recorded".to_owned(),
        )));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            http.clone(),
            Arc::new(RecordingDeleter::returning(200, None)),
        );
        let error = uploader
            .upload(&file(), &StubSecrets::missing())
            .expect_err("missing client id");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("Client ID"));
        assert!(!error.to_string().contains(CLIENT_ID));
        assert!(http.take().is_empty());
    }

    #[test]
    fn an_unreadable_client_id_is_permanent_and_does_not_leak() {
        let http = Arc::new(RecordingHttp::failing(UploadError::Permanent(
            "recorded".to_owned(),
        )));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            http.clone(),
            Arc::new(RecordingDeleter::returning(200, None)),
        );
        let error = uploader
            .upload(&file(), &StubSecrets::failing(SecretError::Unavailable))
            .expect_err("unreadable client id");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!format!("{error} {error:?}").contains(CLIENT_ID));
        assert!(http.take().is_empty());
    }

    #[test]
    fn an_invalid_client_id_is_permanent() {
        let http = Arc::new(RecordingHttp::failing(UploadError::Permanent(
            "recorded".to_owned(),
        )));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            http.clone(),
            Arc::new(RecordingDeleter::returning(200, None)),
        );
        let error = uploader
            .upload(&file(), &StubSecrets::with_client_id("bad id\n"))
            .expect_err("invalid client id");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!error.to_string().contains("bad id"));
        assert!(http.take().is_empty());
    }

    #[test]
    fn unauthorized_is_auth() {
        assert_eq!(
            upload_status(401, r#"{"success":false,"status":401}"#),
            UploadError::Auth
        );
    }

    #[test]
    fn forbidden_is_auth() {
        assert_eq!(
            upload_status(403, r#"{"success":false,"status":403}"#),
            UploadError::Auth
        );
    }

    #[test]
    fn payload_too_large_is_too_large() {
        assert_eq!(upload_status(413, ""), UploadError::TooLarge);
    }

    #[test]
    fn too_many_requests_is_retryable() {
        let error = upload_status(429, "");
        assert!(matches!(error, UploadError::Retryable(_)));
        assert!(error.is_retryable());
    }

    #[test]
    fn server_errors_are_retryable() {
        for status in [500, 502, 503] {
            let error = upload_status(status, "");
            assert!(error.is_retryable(), "{status}");
        }
    }

    #[test]
    fn other_client_errors_are_permanent() {
        for status in [400, 404, 422] {
            let error = upload_status(status, "");
            assert!(matches!(error, UploadError::Permanent(_)), "{status}");
        }
    }

    #[test]
    fn malformed_json_is_permanent_and_does_not_leak_the_body() {
        let error = upload_status(200, "not json, client secret");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!error.to_string().contains("client secret"));
    }

    #[test]
    fn a_failed_envelope_uses_its_status() {
        assert_eq!(
            upload_status(
                200,
                r#"{"success":false,"status":401,"data":{"error":"x"}}"#
            ),
            UploadError::Auth
        );
        assert_eq!(
            upload_status(200, r#"{"success":false,"status":413}"#),
            UploadError::TooLarge
        );
        assert!(upload_status(200, r#"{"success":false,"status":503}"#).is_retryable());
        assert!(matches!(
            upload_status(200, r#"{"success":false,"status":418}"#),
            UploadError::Permanent(_)
        ));
    }

    #[test]
    fn a_successful_envelope_without_a_link_is_permanent() {
        let error = upload_status(200, r#"{"success":true,"status":200,"data":{}}"#);
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("link"));
    }

    #[test]
    fn delete_uses_the_saved_deletehash_and_the_client_id() {
        let deleter = Arc::new(RecordingDeleter::returning(200, None));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            Arc::new(RecordingHttp::failing(UploadError::Permanent(
                "recorded".to_owned(),
            ))),
            deleter.clone(),
        );
        let secrets = StubSecrets::with_client_id(CLIENT_ID);
        uploader
            .delete(
                &DeletionLink::new(DEFAULT_DEST_ID, "https://imgur.com/delete/dh123")
                    .with_token("dh123"),
                &secrets,
            )
            .expect("delete");
        let calls = deleter.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.imgur.com/3/image/dh123");
        assert_eq!(
            calls[0].1,
            vec![("Authorization".to_owned(), format!("Client-ID {CLIENT_ID}"))]
        );
        assert_eq!(
            secrets.requested().as_slice(),
            ["imgur:client_id".to_owned()]
        );
    }

    #[test]
    fn delete_falls_back_to_the_saved_url() {
        let deleter = Arc::new(RecordingDeleter::returning(200, None));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            Arc::new(RecordingHttp::failing(UploadError::Permanent(
                "recorded".to_owned(),
            ))),
            deleter.clone(),
        );
        uploader
            .delete(
                &DeletionLink::new(DEFAULT_DEST_ID, "https://imgur.com/delete/dh123"),
                &StubSecrets::with_client_id(CLIENT_ID),
            )
            .expect("delete");
        assert_eq!(deleter.calls()[0].0, "https://api.imgur.com/3/image/dh123");
    }

    #[test]
    fn delete_treats_404_as_already_gone() {
        let deleter = RecordingDeleter::returning(404, None);
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            Arc::new(RecordingHttp::failing(UploadError::Permanent(
                "recorded".to_owned(),
            ))),
            Arc::new(deleter),
        );
        uploader
            .delete(
                &DeletionLink::new(DEFAULT_DEST_ID, "dh123"),
                &StubSecrets::with_client_id(CLIENT_ID),
            )
            .expect("already gone");
    }

    #[test]
    fn delete_maps_errors() {
        let deleter = RecordingDeleter::returning(401, None);
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            Arc::new(RecordingHttp::failing(UploadError::Permanent(
                "recorded".to_owned(),
            ))),
            Arc::new(deleter),
        );
        assert_eq!(
            uploader
                .delete(
                    &DeletionLink::new(DEFAULT_DEST_ID, "dh123"),
                    &StubSecrets::with_client_id(CLIENT_ID)
                )
                .expect_err("unauthorized"),
            UploadError::Auth
        );

        let deleter = RecordingDeleter::returning(429, Some(Duration::from_secs(2)));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            Arc::new(RecordingHttp::failing(UploadError::Permanent(
                "recorded".to_owned(),
            ))),
            Arc::new(deleter),
        );
        assert_eq!(
            uploader
                .delete(
                    &DeletionLink::new(DEFAULT_DEST_ID, "dh123"),
                    &StubSecrets::with_client_id(CLIENT_ID)
                )
                .expect_err("too many requests"),
            UploadError::Retryable("The server returned HTTP 429, retry after 2 s.".to_owned())
        );
    }

    #[test]
    fn delete_rejects_an_invalid_link_without_sending() {
        let deleter = Arc::new(RecordingDeleter::returning(200, None));
        let uploader = ImgurUploader::new(
            DEFAULT_DEST_ID,
            Arc::new(RecordingHttp::failing(UploadError::Permanent(
                "recorded".to_owned(),
            ))),
            deleter.clone(),
        );
        let error = uploader
            .delete(
                &DeletionLink::new(DEFAULT_DEST_ID, "client secret"),
                &StubSecrets::with_client_id(CLIENT_ID),
            )
            .expect_err("invalid link");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!error.to_string().contains("secret"));
        assert!(deleter.calls().is_empty());
    }

    #[test]
    fn the_deleter_sends_a_delete_request() {
        let server = Server::http("127.0.0.1:0").expect("stub server");
        let base = format!("http://{}", server.server_addr());
        let handle = std::thread::spawn(move || {
            let request = server.recv().expect("stub request");
            let captured = Captured {
                method: request.method().to_string(),
                authorization: header(&request, "Authorization"),
                content_type: String::new(),
                body: Vec::new(),
            };
            let _ = request.respond(StubResponse::from_string("").with_status_code(200));
            captured
        });
        let deleter =
            UreqDeleter::new(HttpTimeout::ThirtySeconds, ProxySetting::None).expect("deleter");
        let response = deleter
            .delete(
                &format!("{base}/3/image/dh123"),
                &[("Authorization".to_owned(), format!("Client-ID {CLIENT_ID}"))],
            )
            .expect("delete response");
        let captured = handle.join().expect("stub server");
        assert_eq!(response.status, 200);
        assert_eq!(captured.method, "DELETE");
        assert_eq!(captured.authorization, format!("Client-ID {CLIENT_ID}"));
    }

    #[test]
    fn errors_do_not_print_the_client_id() {
        assert!(
            !format!("{:?} {:?}", UploadError::Auth, SecretError::Unavailable).contains(CLIENT_ID)
        );
        let error = upload_status(401, CLIENT_ID);
        assert!(!format!("{error} {error:?}").contains(CLIENT_ID));
    }
}
