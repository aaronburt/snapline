use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Proxy};

use crate::error::UploadError;
use crate::http::{HttpTimeout, ProxySetting, USER_AGENT};
use crate::queue::{DeletionLink, SecretStore, UploadFile, Uploaded, Uploader};

pub mod sigv4;

pub const ACCESS_KEY_ID_FIELD: &str = "access_key_id";
pub const SECRET_ACCESS_KEY_FIELD: &str = "secret_access_key";
pub const SESSION_TOKEN_FIELD: &str = "session_token";
pub const DEFAULT_PUBLIC_URL_PATTERN: &str = "https://<bucket>.s3.<region>.amazonaws.com/<key>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3Method {
    Put,
    Delete,
}

impl S3Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Request {
    pub method: S3Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl S3Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn is_success(&self) -> bool {
        is_success(self.status)
    }
}

pub trait S3Transport: Send + Sync {
    fn execute(&self, request: S3Request) -> Result<S3Response, UploadError>;
}

pub struct HttpS3Transport {
    agent: Agent,
}

impl HttpS3Transport {
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

impl S3Transport for HttpS3Transport {
    fn execute(&self, request: S3Request) -> Result<S3Response, UploadError> {
        let method = match request.method {
            S3Method::Put => ureq::http::Method::PUT,
            S3Method::Delete => ureq::http::Method::DELETE,
        };
        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let built = builder
            .body(request.body)
            .map_err(|_| UploadError::Permanent("The request could not be built.".to_owned()))?;
        let response = self
            .agent
            .run(built)
            .map_err(|error| UploadError::from_ureq(&error))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        response
            .into_body()
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|error| UploadError::from_io(&error))?;
        Ok(S3Response {
            status,
            headers,
            body,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Config {
    pub dest_id: String,
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub path_prefix: String,
    pub public_url_pattern: String,
    pub acl_public_read: bool,
    pub path_style: bool,
}

impl S3Config {
    pub fn new(
        dest_id: impl Into<String>,
        bucket: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            dest_id: dest_id.into(),
            bucket: bucket.into(),
            region: region.into(),
            endpoint: None,
            path_prefix: String::new(),
            public_url_pattern: DEFAULT_PUBLIC_URL_PATTERN.to_owned(),
            acl_public_read: false,
            path_style: false,
        }
    }

    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.path_prefix = prefix.into();
        self
    }

    pub fn public_url_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.public_url_pattern = pattern.into();
        self
    }

    pub fn acl_public_read(mut self, enabled: bool) -> Self {
        self.acl_public_read = enabled;
        self
    }

    pub fn path_style(mut self, enabled: bool) -> Self {
        self.path_style = enabled;
        self
    }

    pub fn object_key(&self, file_name: &str) -> String {
        object_key(&self.path_prefix, file_name)
    }

    pub fn public_url(&self, key: &str) -> String {
        self.public_url_pattern
            .replace("<bucket>", &self.bucket)
            .replace("<region>", &self.region)
            .replace("<key>", &sigv4::encode_path(key))
    }
}

pub struct S3Uploader {
    config: S3Config,
    transport: Arc<dyn S3Transport>,
}

impl S3Uploader {
    pub fn new(config: S3Config, transport: Arc<dyn S3Transport>) -> Self {
        Self { config, transport }
    }

    pub fn with_http(
        config: S3Config,
        timeout: HttpTimeout,
        proxy: ProxySetting,
    ) -> Result<Self, UploadError> {
        Ok(Self::new(
            config,
            Arc::new(HttpS3Transport::new(timeout, proxy)?),
        ))
    }

    pub fn config(&self) -> &S3Config {
        &self.config
    }

    fn credentials(&self, secrets: &dyn SecretStore) -> Result<sigv4::Credentials, UploadError> {
        let access_key_id = self.required_secret(secrets, ACCESS_KEY_ID_FIELD)?;
        let secret_access_key = self.required_secret(secrets, SECRET_ACCESS_KEY_FIELD)?;
        let session_token = self.optional_secret(secrets, SESSION_TOKEN_FIELD)?;
        let credentials = sigv4::Credentials::new(access_key_id, secret_access_key);
        Ok(match session_token {
            Some(token) => credentials.with_session_token(token),
            None => credentials,
        })
    }

    fn required_secret(
        &self,
        secrets: &dyn SecretStore,
        field: &str,
    ) -> Result<String, UploadError> {
        self.optional_secret(secrets, field)?.ok_or_else(|| {
            UploadError::Permanent(format!("The S3 destination has no {field} saved."))
        })
    }

    fn optional_secret(
        &self,
        secrets: &dyn SecretStore,
        field: &str,
    ) -> Result<Option<String>, UploadError> {
        secrets.secret(&self.config.dest_id, field)
    }

    fn target(&self, key: &str) -> Result<Target, UploadError> {
        let bucket = self.config.bucket.as_str();
        let region = self.config.region.as_str();
        if let Some(endpoint) = self.config.endpoint.as_deref() {
            let parsed = parse_url(endpoint)?;
            let base = parsed.path.trim_end_matches('/');
            if self.config.path_style {
                return Ok(Target {
                    scheme: parsed.scheme,
                    host: parsed.host,
                    path: format!("{base}/{bucket}/{key}"),
                });
            }
            return Ok(Target {
                scheme: parsed.scheme,
                host: format!("{bucket}.{}", parsed.host),
                path: format!("{base}/{key}"),
            });
        }
        if self.config.path_style {
            return Ok(Target {
                scheme: "https".to_owned(),
                host: format!("s3.{region}.amazonaws.com"),
                path: format!("/{bucket}/{key}"),
            });
        }
        Ok(Target {
            scheme: "https".to_owned(),
            host: format!("{bucket}.s3.{region}.amazonaws.com"),
            path: format!("/{key}"),
        })
    }

    fn signed_headers(
        &self,
        credentials: &sigv4::Credentials,
        amz_date: &str,
        payload_hash: &str,
        mut headers: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        if let Some(token) = credentials.session_token() {
            headers.push(("x-amz-security-token".to_owned(), token.to_owned()));
        }
        headers.push(("x-amz-date".to_owned(), amz_date.to_owned()));
        headers.push(("x-amz-content-sha256".to_owned(), payload_hash.to_owned()));
        headers
    }
}

impl Uploader for S3Uploader {
    fn upload(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        let credentials = self.credentials(secrets)?;
        let key = self.config.object_key(&file.file_name);
        let target = self.target(&key)?;
        let amz_date = sigv4::format_amz_date(now_seconds());
        let payload_hash = sigv4::sha256_hex(&file.bytes);
        let content_type = if file.content_type.trim().is_empty() {
            "application/octet-stream"
        } else {
            file.content_type.trim()
        };
        let mut headers = vec![
            ("host".to_owned(), target.host.clone()),
            ("content-type".to_owned(), content_type.to_owned()),
        ];
        if self.config.acl_public_read {
            headers.push(("x-amz-acl".to_owned(), "public-read".to_owned()));
        }
        let headers = self.signed_headers(&credentials, &amz_date, &payload_hash, headers);
        let pairs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let canonical_path = sigv4::canonical_uri(&target.path);
        let signature = sigv4::sign(
            &credentials,
            &self.config.region,
            "s3",
            &amz_date,
            &sigv4::SignRequest {
                method: "PUT",
                canonical_uri: &canonical_path,
                query: &[],
                headers: &pairs,
                unsigned_headers: &[],
                payload_hash: &payload_hash,
            },
        );
        let mut request_headers = headers;
        request_headers.push(("authorization".to_owned(), signature.authorization));
        let response = self.transport.execute(S3Request {
            method: S3Method::Put,
            url: target.url(),
            headers: request_headers,
            body: file.bytes.clone(),
        })?;
        if !response.is_success() {
            return Err(map_status(response.status));
        }
        let etag = response
            .header("etag")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let link = DeletionLink::new(self.config.dest_id.as_str(), target.url());
        let link = match etag {
            Some(etag) => link.with_token(etag),
            None => link,
        };
        let public_link = if self.config.public_url_pattern == DEFAULT_PUBLIC_URL_PATTERN {
            target.url()
        } else {
            self.config.public_url(&key)
        };
        Ok(Uploaded::new(public_link).with_deletion_link(link))
    }

    fn delete(&self, link: &DeletionLink, secrets: &dyn SecretStore) -> Result<(), UploadError> {
        let credentials = self.credentials(secrets)?;
        let parsed = parse_url(&link.url)?;
        let amz_date = sigv4::format_amz_date(now_seconds());
        let mut headers = vec![("host".to_owned(), parsed.host.clone())];
        if let Some(etag) = link
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            headers.push(("if-match".to_owned(), etag.to_owned()));
        }
        let headers = self.signed_headers(
            &credentials,
            &amz_date,
            sigv4::EMPTY_PAYLOAD_SHA256,
            headers,
        );
        let pairs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let canonical_path = if parsed.path.is_empty() {
            "/".to_owned()
        } else {
            parsed.path.clone()
        };
        let query: Vec<(&str, &str)> = parsed
            .query
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let signature = sigv4::sign(
            &credentials,
            &self.config.region,
            "s3",
            &amz_date,
            &sigv4::SignRequest {
                method: "DELETE",
                canonical_uri: &canonical_path,
                query: &query,
                headers: &pairs,
                unsigned_headers: &[],
                payload_hash: sigv4::EMPTY_PAYLOAD_SHA256,
            },
        );
        let mut request_headers = headers;
        request_headers.push(("authorization".to_owned(), signature.authorization));
        let response = self.transport.execute(S3Request {
            method: S3Method::Delete,
            url: link.url.clone(),
            headers: request_headers,
            body: Vec::new(),
        })?;
        if response.is_success() {
            Ok(())
        } else {
            Err(map_status(response.status))
        }
    }
}

struct Target {
    scheme: String,
    host: String,
    path: String,
}

impl Target {
    fn url(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme,
            self.host,
            sigv4::canonical_uri(&self.path)
        )
    }
}

struct ParsedUrl {
    scheme: String,
    host: String,
    path: String,
    query: Vec<(String, String)>,
}

fn parse_url(url: &str) -> Result<ParsedUrl, UploadError> {
    let invalid = || UploadError::Permanent("The S3 address is not valid.".to_owned());
    let (scheme, rest) = url.split_once("://").ok_or_else(invalid)?;
    if scheme != "http" && scheme != "https" {
        return Err(invalid());
    }
    let (authority, remainder) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(invalid());
    }
    let (path, query) = match remainder.split_once('?') {
        Some((path, query)) => (path.to_owned(), parse_query(query)),
        None => (remainder.to_owned(), Vec::new()),
    };
    Ok(ParsedUrl {
        scheme: scheme.to_owned(),
        host: authority.to_owned(),
        path,
        query,
    })
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) => (percent_decode(name), percent_decode(value)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn is_success(status: u16) -> bool {
    (200..=299).contains(&status)
}

fn map_status(status: u16) -> UploadError {
    match status {
        401 | 403 => UploadError::Auth,
        404 => UploadError::Permanent("The bucket or object was not found.".to_owned()),
        413 => UploadError::TooLarge,
        408 | 429 | 500..=599 => UploadError::from_status(status),
        _ => UploadError::Permanent(format!("The server returned HTTP {status}.")),
    }
}

pub fn object_key(prefix: &str, file_name: &str) -> String {
    let prefix = sanitise_prefix(prefix);
    let name = sanitise_file_name(file_name);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn sanitise_prefix(prefix: &str) -> String {
    let mut sanitised = String::with_capacity(prefix.len());
    for character in prefix.trim_matches('/').chars() {
        let character =
            if character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '_' | '-') {
                character
            } else {
                '_'
            };
        if character == '/' && sanitised.ends_with('/') {
            continue;
        }
        sanitised.push(character);
    }
    sanitised.trim_matches('/').to_owned()
}

fn sanitise_file_name(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let mut sanitised = String::with_capacity(base.len());
    for character in base.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            sanitised.push(character);
        } else {
            sanitised.push('_');
        }
    }
    if sanitised.is_empty() || sanitised.chars().all(|character| character == '.') {
        "file".to_owned()
    } else {
        sanitised
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use crate::queue::SecretError;

    use super::*;

    struct FakeSecrets {
        values: HashMap<String, Vec<u8>>,
    }

    impl FakeSecrets {
        fn new() -> Self {
            Self {
                values: HashMap::new(),
            }
        }

        fn with(mut self, field: &str, value: &str) -> Self {
            self.values
                .insert(field.to_owned(), value.as_bytes().to_vec());
            self
        }
    }

    impl SecretStore for FakeSecrets {
        fn get(&self, dest_id: &str, field: &str) -> Result<Option<Vec<u8>>, SecretError> {
            if dest_id != "s3" {
                return Ok(None);
            }
            Ok(self.values.get(field).cloned())
        }
    }

    struct FakeTransport {
        requests: Mutex<Vec<S3Request>>,
        responses: Mutex<VecDeque<S3Response>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<S3Response>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into()),
            }
        }

        fn requests(&self) -> Vec<S3Request> {
            self.requests.lock().expect("lock").clone()
        }
    }

    impl S3Transport for FakeTransport {
        fn execute(&self, request: S3Request) -> Result<S3Response, UploadError> {
            self.requests.lock().expect("lock").push(request);
            Ok(self
                .responses
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_else(|| ok_response(Vec::new())))
        }
    }

    fn ok_response(headers: Vec<(String, String)>) -> S3Response {
        S3Response {
            status: 200,
            headers,
            body: Vec::new(),
        }
    }

    fn config() -> S3Config {
        S3Config::new("s3", "my-bucket", "eu-west-2")
    }

    fn secrets() -> FakeSecrets {
        FakeSecrets::new()
            .with(ACCESS_KEY_ID_FIELD, "AKIDEXAMPLE")
            .with(SECRET_ACCESS_KEY_FIELD, "top-secret-key")
    }

    fn file() -> UploadFile {
        UploadFile::new(
            "C:\\captures\\Screen Shot!.PNG",
            "Screen Shot!.PNG",
            "image/png",
            vec![1_u8, 2, 3],
        )
    }

    fn header(request: &S3Request, name: &str) -> String {
        request
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }

    fn uploader(config: S3Config) -> (S3Uploader, Arc<FakeTransport>) {
        let transport = Arc::new(FakeTransport::new(vec![ok_response(vec![(
            "ETag".to_owned(),
            "\"etag-1\"".to_owned(),
        )])]));
        (S3Uploader::new(config, transport.clone()), transport)
    }

    #[test]
    fn upload_uses_the_virtual_host_style_and_captures_the_etag() {
        let (uploader, transport) = uploader(config().path_prefix("shots").acl_public_read(true));
        let uploaded = uploader.upload(&file(), &secrets()).expect("upload");
        assert_eq!(
            uploaded.link,
            "https://my-bucket.s3.eu-west-2.amazonaws.com/shots/Screen_Shot_.PNG"
        );
        let link = uploaded.deletion_link.expect("deletion link");
        assert_eq!(link.token.as_deref(), Some("\"etag-1\""));
        assert_eq!(link.url, uploaded.link);
        let request = &transport.requests()[0];
        assert_eq!(request.method, S3Method::Put);
        assert_eq!(request.url, uploaded.link);
        assert_eq!(
            header(request, "host"),
            "my-bucket.s3.eu-west-2.amazonaws.com"
        );
        assert_eq!(header(request, "content-type"), "image/png");
        assert_eq!(header(request, "x-amz-acl"), "public-read");
        assert_eq!(
            header(request, "x-amz-content-sha256"),
            sigv4::sha256_hex(&[1_u8, 2, 3])
        );
        assert!(header(request, "authorization")
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"));
        assert!(header(request, "authorization").contains("content-type"));
        assert!(!format!("{request:?}").contains("top-secret-key"));
    }

    #[test]
    fn upload_uses_path_style_for_the_default_endpoint_when_asked() {
        let (uploader, transport) = uploader(config().path_style(true));
        let uploaded = uploader.upload(&file(), &secrets()).expect("upload");
        assert_eq!(
            uploaded.link,
            "https://s3.eu-west-2.amazonaws.com/my-bucket/Screen_Shot_.PNG"
        );
        let request = &transport.requests()[0];
        assert_eq!(header(request, "host"), "s3.eu-west-2.amazonaws.com");
        assert_eq!(request.url, uploaded.link);
    }

    #[test]
    fn a_custom_endpoint_supports_path_style_and_a_base_path() {
        let (uploader, transport) = uploader(
            config()
                .endpoint("https://s3.example.test:9000/base")
                .path_style(true),
        );
        let uploaded = uploader.upload(&file(), &secrets()).expect("upload");
        assert_eq!(
            uploaded.link,
            "https://s3.example.test:9000/base/my-bucket/Screen_Shot_.PNG"
        );
        let request = &transport.requests()[0];
        assert_eq!(header(request, "host"), "s3.example.test:9000");
    }

    #[test]
    fn a_custom_endpoint_supports_virtual_host_style() {
        let (uploader, transport) = uploader(config().endpoint("https://s3.example.test"));
        let uploaded = uploader.upload(&file(), &secrets()).expect("upload");
        assert_eq!(
            uploaded.link,
            "https://my-bucket.s3.example.test/Screen_Shot_.PNG"
        );
        let request = &transport.requests()[0];
        assert_eq!(header(request, "host"), "my-bucket.s3.example.test");
    }

    #[test]
    fn the_public_url_pattern_can_point_at_a_cdn() {
        let (uploader, _transport) = uploader(
            config()
                .path_prefix("shots")
                .public_url_pattern("https://cdn.example.test/<bucket>/<region>/<key>"),
        );
        let uploaded = uploader.upload(&file(), &secrets()).expect("upload");
        assert_eq!(
            uploaded.link,
            "https://cdn.example.test/my-bucket/eu-west-2/shots/Screen_Shot_.PNG"
        );
    }

    #[test]
    fn the_default_public_url_pattern_is_aws_shaped() {
        let uploaded_config = config();
        assert_eq!(
            uploaded_config.public_url("shots/a.png"),
            "https://my-bucket.s3.eu-west-2.amazonaws.com/shots/a.png"
        );
    }

    #[test]
    fn deletion_sends_the_saved_etag_as_if_match() {
        let transport = Arc::new(FakeTransport::new(vec![S3Response {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        }]));
        let uploader = S3Uploader::new(config(), transport.clone());
        let link = DeletionLink::new(
            "s3",
            "https://my-bucket.s3.eu-west-2.amazonaws.com/shots/a.png",
        )
        .with_token("\"etag-1\"");
        uploader.delete(&link, &secrets()).expect("delete");
        let request = &transport.requests()[0];
        assert_eq!(request.method, S3Method::Delete);
        assert_eq!(request.url, link.url);
        assert_eq!(header(request, "if-match"), "\"etag-1\"");
        assert!(header(request, "authorization").contains("if-match"));
        assert_eq!(
            header(request, "x-amz-content-sha256"),
            sigv4::EMPTY_PAYLOAD_SHA256
        );
    }

    #[test]
    fn deletion_signs_a_version_query() {
        let transport = Arc::new(FakeTransport::new(vec![S3Response {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        }]));
        let uploader = S3Uploader::new(config(), transport.clone());
        let link = DeletionLink::new(
            "s3",
            "https://my-bucket.s3.eu-west-2.amazonaws.com/shots/a.png?versionId=abc%20123",
        );
        uploader.delete(&link, &secrets()).expect("delete");
        let request = &transport.requests()[0];
        assert_eq!(request.url, link.url);
        assert_eq!(
            header(request, "host"),
            "my-bucket.s3.eu-west-2.amazonaws.com"
        );
        assert!(header(request, "authorization").starts_with("AWS4-HMAC-SHA256"));
    }

    #[test]
    fn http_statuses_map_to_the_documented_error_kinds() {
        for (status, expected) in [
            (401, "auth"),
            (403, "auth"),
            (404, "permanent"),
            (413, "too large"),
            (429, "retryable"),
            (500, "retryable"),
            (503, "retryable"),
            (400, "permanent"),
        ] {
            let transport = Arc::new(FakeTransport::new(vec![S3Response {
                status,
                headers: Vec::new(),
                body: Vec::new(),
            }]));
            let uploader = S3Uploader::new(config(), transport);
            let error = uploader.upload(&file(), &secrets()).expect_err("status");
            let kind = match error {
                UploadError::Auth => "auth",
                UploadError::TooLarge => "too large",
                UploadError::Retryable(_) => "retryable",
                UploadError::Permanent(_) => "permanent",
            };
            assert_eq!(kind, expected, "{status}");
        }
    }

    #[test]
    fn missing_credentials_are_permanent_and_never_leak_a_secret() {
        let (uploader, _transport) = uploader(config());
        let secrets = FakeSecrets::new().with(SECRET_ACCESS_KEY_FIELD, "top-secret-key");
        let error = uploader.upload(&file(), &secrets).expect_err("missing key");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!error.to_string().contains("top-secret-key"));
    }

    #[test]
    fn object_keys_are_sanitised_and_prefixed() {
        assert_eq!(
            object_key("shots//2026", "..\\..\\Screen Shot 1.PNG"),
            "shots/2026/Screen_Shot_1.PNG"
        );
        assert_eq!(object_key("", "shot.png"), "shot.png");
        assert_eq!(object_key("/shots/", "shot.png"), "shots/shot.png");
        assert_eq!(object_key("", "..."), "file");
        assert_eq!(
            object_key("bad prefix!", "shot.png"),
            "bad_prefix_/shot.png"
        );
    }

    #[test]
    fn a_session_token_is_sent_when_saved() {
        let transport = Arc::new(FakeTransport::new(vec![ok_response(Vec::new())]));
        let uploader = S3Uploader::new(config(), transport.clone());
        let secrets = secrets().with(SESSION_TOKEN_FIELD, "session-token-value");
        uploader.upload(&file(), &secrets).expect("upload");
        let request = &transport.requests()[0];
        assert_eq!(
            header(request, "x-amz-security-token"),
            "session-token-value"
        );
        assert!(header(request, "authorization").contains("x-amz-security-token"));
        assert!(!format!("{request:?}").contains("top-secret-key"));
    }

    #[test]
    fn an_empty_content_type_falls_back_to_octet_stream() {
        let (uploader, transport) = uploader(config());
        let file = UploadFile::new("C:\\captures\\shot.png", "shot.png", "  ", vec![1_u8]);
        uploader.upload(&file, &secrets()).expect("upload");
        assert_eq!(
            header(&transport.requests()[0], "content-type"),
            "application/octet-stream"
        );
    }
}
