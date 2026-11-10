use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::UploadError;
use crate::http::{HttpClient, HttpTimeout, HttpTransport, ProxySetting, Request, USER_AGENT};
use crate::queue::{
    DeletionLink, SecretStore, UploadFile, Uploaded, Uploader, MAX_BUFFERED_UPLOAD_BYTES,
};

pub const DEFAULT_DEST_ID: &str = "gist";
pub const TOKEN_FIELD: &str = "token";
pub const API_BASE: &str = "https://api.github.com";
pub const ACCEPT: &str = "application/vnd.github+json";
pub const AUTHORIZATION_PREFIX: &str = "Bearer";
pub const SECRET_TARGET_PREFIX: &str = "SnapLine:";
pub const DELETE_UNSUPPORTED: &str =
    "Deleting a gist is not supported. Delete it on GitHub instead.";

#[derive(Serialize)]
struct GistRequest<'a> {
    description: &'a str,
    public: bool,
    files: BTreeMap<&'a str, GistFileContent<'a>>,
}

#[derive(Serialize)]
struct GistFileContent<'a> {
    content: &'a str,
}

#[derive(Deserialize)]
struct GistResponse {
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, GistFileResponse>,
}

#[derive(Deserialize)]
struct GistFileResponse {
    #[serde(default)]
    raw_url: Option<String>,
}

pub struct GistUploader {
    dest_id: String,
    public: bool,
    api_base: String,
    http: Arc<dyn HttpTransport>,
}

impl GistUploader {
    pub fn new(dest_id: impl Into<String>, public: bool, http: Arc<dyn HttpTransport>) -> Self {
        Self {
            dest_id: dest_id.into(),
            public,
            api_base: API_BASE.to_owned(),
            http,
        }
    }

    pub fn from_settings(
        dest_id: impl Into<String>,
        public: bool,
        timeout: HttpTimeout,
        proxy: ProxySetting,
    ) -> Result<Self, UploadError> {
        let http = HttpClient::new(timeout, proxy)?;
        Ok(Self::new(dest_id, public, Arc::new(http)))
    }

    pub fn gist(http: Arc<dyn HttpTransport>) -> Self {
        Self::new(DEFAULT_DEST_ID, false, http)
    }

    pub fn dest_id(&self) -> &str {
        &self.dest_id
    }

    pub fn is_public(&self) -> bool {
        self.public
    }

    pub fn token_key(&self) -> String {
        format!("{SECRET_TARGET_PREFIX}{}:{TOKEN_FIELD}", self.dest_id)
    }

    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into().trim_end_matches('/').to_owned();
        self
    }

    fn token(&self, secrets: &dyn SecretStore) -> Result<String, UploadError> {
        match secrets.get(&self.dest_id, TOKEN_FIELD) {
            Ok(Some(mut bytes)) => {
                let value = std::str::from_utf8(&bytes)
                    .ok()
                    .filter(|value| {
                        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic())
                    })
                    .map(str::to_owned);
                bytes.fill(0);
                value.ok_or_else(|| {
                    UploadError::Permanent("The saved GitHub token is not valid.".to_owned())
                })
            }
            Ok(None) => Err(UploadError::Permanent(
                "No GitHub token is saved. Add one in Settings.".to_owned(),
            )),
            Err(_) => Err(UploadError::Permanent(
                "The saved GitHub token could not be read.".to_owned(),
            )),
        }
    }

    fn upload_text(
        &self,
        file_name: &str,
        content: &str,
        token: &str,
    ) -> Result<Uploaded, UploadError> {
        if file_name.trim().is_empty() {
            return Err(UploadError::Permanent(
                "The file name is not valid.".to_owned(),
            ));
        }
        let mut files = BTreeMap::new();
        files.insert(file_name, GistFileContent { content });
        let body = GistRequest {
            description: file_name,
            public: self.public,
            files,
        };
        let request = Request::post(format!("{}/gists", self.api_base))
            .header("Authorization", format!("{AUTHORIZATION_PREFIX} {token}"))
            .header("Accept", ACCEPT)
            .header("User-Agent", USER_AGENT)
            .json(&body)?;
        let response = self.http.send(request)?;
        let gist: GistResponse = response.json()?;
        let page_link = gist.html_url.filter(|value| !value.is_empty());
        let direct_link = gist
            .files
            .get(file_name)
            .and_then(|file| file.raw_url.clone())
            .or_else(|| gist.files.values().find_map(|file| file.raw_url.clone()))
            .filter(|value| !value.is_empty());
        match (direct_link, page_link) {
            (Some(direct), Some(page)) => Ok(Uploaded::new(direct).with_page_link(page)),
            (Some(direct), None) => Ok(Uploaded::new(direct)),
            (None, Some(page)) => Ok(Uploaded::new(page.clone()).with_page_link(page)),
            (None, None) => Err(UploadError::Permanent(
                "GitHub did not return a link.".to_owned(),
            )),
        }
    }
}

impl Uploader for GistUploader {
    fn upload(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        let bytes = file.read_all(MAX_BUFFERED_UPLOAD_BYTES)?;
        let content = std::str::from_utf8(&bytes).map_err(|_| {
            UploadError::Permanent(
                "The file is not text and cannot be uploaded to a gist.".to_owned(),
            )
        })?;
        let token = self.token(secrets)?;
        self.upload_text(&file.file_name, content, &token)
    }

    fn delete(&self, _link: &DeletionLink, _secrets: &dyn SecretStore) -> Result<(), UploadError> {
        Err(UploadError::Permanent(DELETE_UNSUPPORTED.to_owned()))
    }
}

impl fmt::Debug for GistUploader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GistUploader")
            .field("dest_id", &self.dest_id)
            .field("public", &self.public)
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};
    use std::thread::JoinHandle;

    use tiny_http::{Request as StubRequest, Response as StubResponse, Server};

    use super::*;
    use crate::queue::SecretError;

    const TOKEN: &str = "ghp_abc123def456";

    struct StubSecrets {
        value: Option<Vec<u8>>,
        error: Option<SecretError>,
        requested: Mutex<Vec<String>>,
    }

    impl StubSecrets {
        fn with_token(value: &str) -> Self {
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
    }

    impl RecordingHttp {
        fn new() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
            }
        }

        fn take(&self) -> Vec<Request> {
            std::mem::take(&mut *self.requests.lock().expect("requests"))
        }
    }

    impl HttpTransport for RecordingHttp {
        fn execute(&self, request: Request) -> Result<crate::http::Response, UploadError> {
            self.requests.lock().expect("requests").push(request);
            Err(UploadError::Permanent("recorded".to_owned()))
        }
    }

    struct Captured {
        method: String,
        authorization: String,
        accept: String,
        user_agent: String,
        content_type: String,
    }

    fn serve(status: u16, body: &'static str) -> (String, JoinHandle<Captured>) {
        let server = Server::http("127.0.0.1:0").expect("stub server");
        let base = format!("http://{}", server.server_addr());
        let handle = std::thread::spawn(move || {
            let request = server.recv().expect("stub request");
            let captured = Captured {
                method: request.method().to_string(),
                authorization: header(&request, "Authorization"),
                accept: header(&request, "Accept"),
                user_agent: header(&request, "User-Agent"),
                content_type: header(&request, "Content-Type"),
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

    fn file(name: &str, content: &str) -> UploadFile {
        UploadFile::new(
            PathBuf::from(format!("captures/{name}")),
            name,
            "text/plain",
            content.as_bytes().to_vec(),
        )
    }

    fn recording_uploader() -> (Arc<RecordingHttp>, GistUploader) {
        let http = Arc::new(RecordingHttp::new());
        let uploader = GistUploader::new(DEFAULT_DEST_ID, true, http.clone());
        (http, uploader)
    }

    fn upload_status(status: u16, body: &'static str) -> UploadError {
        let (base, handle) = serve(status, body);
        let uploader = GistUploader::new(DEFAULT_DEST_ID, false, test_client()).with_api_base(base);
        let error = uploader
            .upload(&file("note.txt", "hello"), &StubSecrets::with_token(TOKEN))
            .expect_err("upload error");
        handle.join().expect("stub server");
        error
    }

    #[test]
    fn the_request_matches_the_github_gist_api() {
        let (http, uploader) = recording_uploader();
        let secrets = StubSecrets::with_token(TOKEN);
        let _ = uploader.upload(&file("note.txt", "hello"), &secrets);
        let requests = http.take();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method(), crate::http::Method::Post);
        assert_eq!(request.url(), "https://api.github.com/gists");
        assert_eq!(
            header_value(request, "Authorization"),
            format!("{AUTHORIZATION_PREFIX} {TOKEN}")
        );
        assert_eq!(header_value(request, "Accept"), ACCEPT);
        assert_eq!(header_value(request, "User-Agent"), USER_AGENT);
        assert_eq!(header_value(request, "Content-Type"), "application/json");
        let body: serde_json::Value =
            serde_json::from_slice(request.body_bytes()).expect("json body");
        assert_eq!(
            body,
            serde_json::json!({
                "description": "note.txt",
                "public": true,
                "files": {"note.txt": {"content": "hello"}}
            })
        );
    }

    fn header_value(request: &Request, name: &str) -> String {
        request
            .headers()
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    }

    #[test]
    fn the_token_key_matches_the_credential_manager_plan() {
        let (_http, uploader) = recording_uploader();
        assert_eq!(uploader.token_key(), "SnapLine:gist:token");
        let secrets = StubSecrets::with_token(TOKEN);
        let _ = uploader.upload(&file("note.txt", "hello"), &secrets);
        assert_eq!(secrets.requested().as_slice(), ["gist:token".to_owned()]);
        let other = GistUploader::new("gist-2", false, Arc::new(RecordingHttp::new()));
        assert_eq!(other.token_key(), "SnapLine:gist-2:token");
    }

    #[test]
    fn the_public_toggle_changes_the_body() {
        let (http, uploader) = recording_uploader();
        let _ = uploader.upload(&file("note.txt", "hello"), &StubSecrets::with_token(TOKEN));
        let body: serde_json::Value =
            serde_json::from_slice(http.take()[0].body_bytes()).expect("json body");
        assert_eq!(body["public"], true);

        let http = Arc::new(RecordingHttp::new());
        let secret_gist = GistUploader::new(DEFAULT_DEST_ID, false, http.clone());
        let _ = secret_gist.upload(&file("note.txt", "hello"), &StubSecrets::with_token(TOKEN));
        let body: serde_json::Value =
            serde_json::from_slice(http.take()[0].body_bytes()).expect("json body");
        assert_eq!(body["public"], false);
        assert!(!secret_gist.is_public());
    }

    #[test]
    fn a_successful_response_returns_the_raw_and_page_links() {
        let (base, handle) = serve(
            201,
            r#"{"html_url":"https://gist.github.com/abc123","files":{"note.txt":{"raw_url":"https://gist.githubusercontent.com/u/abc123/raw/note.txt"}}}"#,
        );
        let uploader = GistUploader::new(DEFAULT_DEST_ID, false, test_client()).with_api_base(base);
        let uploaded = uploader
            .upload(&file("note.txt", "hello"), &StubSecrets::with_token(TOKEN))
            .expect("upload");
        let captured = handle.join().expect("stub server");
        assert_eq!(captured.method, "POST");
        assert_eq!(captured.authorization, format!("Bearer {TOKEN}"));
        assert_eq!(captured.accept, ACCEPT);
        assert_eq!(captured.user_agent, USER_AGENT);
        assert_eq!(captured.content_type, "application/json");
        assert_eq!(
            uploaded.link,
            "https://gist.githubusercontent.com/u/abc123/raw/note.txt"
        );
        assert_eq!(
            uploaded.page_link.as_deref(),
            Some("https://gist.github.com/abc123")
        );
        assert_eq!(uploaded.deletion_link, None);
    }

    #[test]
    fn a_missing_raw_url_falls_back_to_the_page_link() {
        let (base, handle) = serve(
            201,
            r#"{"html_url":"https://gist.github.com/abc123","files":{"note.txt":{}}}"#,
        );
        let uploader = GistUploader::new(DEFAULT_DEST_ID, false, test_client()).with_api_base(base);
        let uploaded = uploader
            .upload(&file("note.txt", "hello"), &StubSecrets::with_token(TOKEN))
            .expect("upload");
        handle.join().expect("stub server");
        assert_eq!(uploaded.link, "https://gist.github.com/abc123");
        assert_eq!(
            uploaded.page_link.as_deref(),
            Some("https://gist.github.com/abc123")
        );
    }

    #[test]
    fn a_response_without_links_is_permanent() {
        let (base, handle) = serve(201, r#"{"files":{"note.txt":{}}}"#);
        let uploader = GistUploader::new(DEFAULT_DEST_ID, false, test_client()).with_api_base(base);
        let error = uploader
            .upload(&file("note.txt", "hello"), &StubSecrets::with_token(TOKEN))
            .expect_err("no link");
        handle.join().expect("stub server");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("link"));
    }

    #[test]
    fn unauthorized_and_forbidden_are_auth() {
        assert_eq!(upload_status(401, ""), UploadError::Auth);
        assert_eq!(upload_status(403, ""), UploadError::Auth);
    }

    #[test]
    fn not_found_and_unprocessable_are_permanent() {
        assert!(matches!(upload_status(404, ""), UploadError::Permanent(_)));
        assert!(matches!(upload_status(422, ""), UploadError::Permanent(_)));
    }

    #[test]
    fn payload_too_large_is_too_large() {
        assert_eq!(upload_status(413, ""), UploadError::TooLarge);
    }

    #[test]
    fn too_many_requests_and_server_errors_are_retryable() {
        assert!(upload_status(429, "").is_retryable());
        for status in [500, 502, 503, 599] {
            assert!(upload_status(status, "").is_retryable(), "{status}");
        }
    }

    #[test]
    fn malformed_json_is_permanent_and_does_not_leak_the_body() {
        let error = upload_status(201, "not json, ghp_abc123def456");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!error.to_string().contains(TOKEN));
    }

    #[test]
    fn a_missing_token_is_permanent_and_sends_nothing() {
        let (http, uploader) = recording_uploader();
        let error = uploader
            .upload(&file("note.txt", "hello"), &StubSecrets::missing())
            .expect_err("missing token");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("token"));
        assert!(http.take().is_empty());
    }

    #[test]
    fn an_unreadable_token_is_permanent_and_sends_nothing() {
        let (http, uploader) = recording_uploader();
        let error = uploader
            .upload(
                &file("note.txt", "hello"),
                &StubSecrets::failing(SecretError::Unavailable),
            )
            .expect_err("unreadable token");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(http.take().is_empty());
    }

    #[test]
    fn an_invalid_token_is_permanent_and_does_not_leak() {
        let (http, uploader) = recording_uploader();
        let error = uploader
            .upload(
                &file("note.txt", "hello"),
                &StubSecrets::with_token("bad token\n"),
            )
            .expect_err("invalid token");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!format!("{error} {error:?}").contains("bad token"));
        assert!(http.take().is_empty());
    }

    #[test]
    fn non_text_content_is_permanent() {
        let (http, uploader) = recording_uploader();
        let binary = UploadFile::new(
            PathBuf::from("captures/shot.png"),
            "shot.png",
            "image/png",
            vec![0xff_u8, 0xfe, 0x00],
        );
        let error = uploader
            .upload(&binary, &StubSecrets::with_token(TOKEN))
            .expect_err("binary");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(http.take().is_empty());
    }

    #[test]
    fn delete_is_unsupported_and_does_not_read_the_token() {
        let (http, uploader) = recording_uploader();
        let secrets = StubSecrets::with_token(TOKEN);
        let error = uploader
            .delete(
                &DeletionLink::new(DEFAULT_DEST_ID, "https://gist.github.com/abc123"),
                &secrets,
            )
            .expect_err("delete unsupported");
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(error.to_string().contains("not supported"));
        assert!(!error.to_string().contains(TOKEN));
        assert!(secrets.requested().is_empty());
        assert!(http.take().is_empty());
    }

    #[test]
    fn errors_and_debug_never_print_the_token() {
        let (_http, uploader) = recording_uploader();
        assert!(!format!("{uploader:?}").contains(TOKEN));
        let error = upload_status(401, TOKEN);
        assert!(!format!("{error} {error:?}").contains(TOKEN));
    }
}
