use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::error::UploadError;
use crate::http::{HttpTransport, Method, Multipart, Request};
use crate::queue::{
    DeletionLink, SecretStore, UploadFile, Uploaded, Uploader, MAX_BUFFERED_UPLOAD_BYTES,
};

pub mod sxcu;
pub mod template;

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
const MAX_ERROR_CHARS: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomUploaded {
    pub uploaded: Uploaded,
    pub thumbnail_link: Option<String>,
}

pub fn header_secret_field(name: &str) -> String {
    let mut field = String::with_capacity(7 + name.len());
    field.push_str("header_");
    field.push_str(&name.to_ascii_lowercase());
    field
}

pub struct CustomUploader {
    dest_id: String,
    config: sxcu::CustomUploaderConfig,
    transport: Arc<dyn Transport>,
}

impl CustomUploader {
    pub fn new(
        dest_id: impl Into<String>,
        config: sxcu::CustomUploaderConfig,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            dest_id: dest_id.into(),
            config,
            transport: Arc::new(HttpAdapter(transport)),
        }
    }

    #[cfg(test)]
    fn with_transport(
        dest_id: impl Into<String>,
        config: sxcu::CustomUploaderConfig,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            dest_id: dest_id.into(),
            config,
            transport,
        }
    }

    pub fn config(&self) -> &sxcu::CustomUploaderConfig {
        &self.config
    }

    pub fn dest_id(&self) -> &str {
        &self.dest_id
    }

    pub fn test_upload(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        self.upload_custom(file, secrets)
            .map(|result| result.uploaded)
    }

    pub fn export(
        &self,
        include_header_values: bool,
        secrets: &dyn SecretStore,
    ) -> Result<String, UploadError> {
        let mut values = sxcu::HeaderValues::new();
        if include_header_values {
            for header in &self.config.headers {
                if let Some(value) = secret_text(secrets, &self.dest_id, &header.name)? {
                    values.insert(header.name.clone(), value);
                }
            }
        }
        sxcu::export(&self.config, include_header_values, &values)
            .map_err(|error| UploadError::Permanent(error.to_string()))
    }

    pub fn upload_custom(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<CustomUploaded, UploadError> {
        let random = template::random_token();
        let request = self.build_request(file, secrets, &random)?;
        let reply = self.transport.execute(request)?;
        if !reply.is_success() {
            return Err(response_error(
                &self.config,
                &reply,
                &file.file_name,
                &random,
            ));
        }
        let text = reply.text()?;
        let json = serde_json::from_str::<Value>(&text).ok();
        let context = template::TemplateContext {
            file_name: &file.file_name,
            random: &random,
            response: Some(&text),
            response_json: json.as_ref(),
        };
        let link = expand_link(&self.config.link, &context)?;
        let thumbnail = expand_optional(self.config.thumbnail.as_deref(), &context)?;
        let deletion = expand_optional(self.config.deletion.as_deref(), &context)?;
        let uploaded = match deletion {
            Some(url) => {
                Uploaded::new(link).with_deletion_link(DeletionLink::new(self.dest_id.clone(), url))
            }
            None => Uploaded::new(link),
        };
        Ok(CustomUploaded {
            uploaded,
            thumbnail_link: thumbnail,
        })
    }

    fn build_request(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
        random: &str,
    ) -> Result<Request, UploadError> {
        let context = template::TemplateContext {
            file_name: &file.file_name,
            random,
            response: None,
            response_json: None,
        };
        let url =
            template::expand(&self.config.request_url, &context, true).map_err(template_error)?;
        let url = outgoing_url(&url)?;
        let method = match self.config.method {
            Method::Post => Request::post(url),
            Method::Put => Request::put(url),
            Method::Patch => Request::patch(url),
            Method::Get => {
                return Err(UploadError::Permanent(
                    "The request method must be POST, PUT or PATCH.".to_owned(),
                ));
            }
        };
        let request = self.apply_headers(method, secrets)?;
        apply_body(request, &self.config, file)
    }

    fn apply_headers(
        &self,
        request: Request,
        secrets: &dyn SecretStore,
    ) -> Result<Request, UploadError> {
        let mut request = request;
        for header in &self.config.headers {
            if let Some(value) = secret_text(secrets, &self.dest_id, &header.name)? {
                if !value.is_empty() {
                    request = request.header(header.name.clone(), value);
                }
            }
        }
        Ok(request)
    }
}

impl Uploader for CustomUploader {
    fn upload(
        &self,
        file: &UploadFile,
        secrets: &dyn SecretStore,
    ) -> Result<Uploaded, UploadError> {
        self.upload_custom(file, secrets)
            .map(|result| result.uploaded)
    }

    fn delete(&self, link: &DeletionLink, secrets: &dyn SecretStore) -> Result<(), UploadError> {
        let url = outgoing_url(&link.url)?;
        let request = self.apply_headers(Request::get(url), secrets)?;
        let reply = self.transport.execute(request)?;
        if reply.is_success() {
            Ok(())
        } else {
            Err(UploadError::from_status_with_retry_after(
                reply.status,
                reply.retry_after(),
            ))
        }
    }
}

trait Transport: Send + Sync {
    fn execute(&self, request: Request) -> Result<HttpReply, UploadError>;
}

struct HttpAdapter(Arc<dyn HttpTransport>);

impl Transport for HttpAdapter {
    fn execute(&self, request: Request) -> Result<HttpReply, UploadError> {
        self.0.execute(request).map(HttpReply::from)
    }
}

struct HttpReply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpReply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn retry_after(&self) -> Option<Duration> {
        self.header("retry-after")?
            .trim()
            .parse::<u64>()
            .ok()
            .map(Duration::from_secs)
    }

    fn text(&self) -> Result<String, UploadError> {
        String::from_utf8(self.body.clone()).map_err(|_| {
            UploadError::Permanent("The server response is not valid UTF-8.".to_owned())
        })
    }

    fn is_success(&self) -> bool {
        (200..=299).contains(&self.status)
    }
}

impl From<crate::http::Response> for HttpReply {
    fn from(response: crate::http::Response) -> Self {
        Self {
            status: response.status(),
            headers: response.headers().to_vec(),
            body: response.body().to_vec(),
        }
    }
}

fn apply_body(
    request: Request,
    config: &sxcu::CustomUploaderConfig,
    file: &UploadFile,
) -> Result<Request, UploadError> {
    let field = if config.file_form_field.trim().is_empty() {
        sxcu::DEFAULT_FILE_FORM_FIELD
    } else {
        config.file_form_field.as_str()
    };
    let content_type = content_type(file);
    let bytes = file.read_all(MAX_BUFFERED_UPLOAD_BYTES)?;
    match config.body {
        sxcu::BodyType::MultipartFormData => Ok(request.multipart(Multipart::new().file(
            field,
            &file.file_name,
            content_type,
            bytes.clone(),
        ))),
        sxcu::BodyType::Binary => Ok(request.body(content_type, bytes)),
        sxcu::BodyType::FormUrlEncoded => {
            let encoded = base64_encode(&bytes);
            Ok(request.form(&[(field, &encoded)]))
        }
        sxcu::BodyType::Json => {
            let mut object = serde_json::Map::new();
            object.insert(field.to_owned(), Value::String(base64_encode(&bytes)));
            request.json(&Value::Object(object))
        }
    }
}

fn content_type(file: &UploadFile) -> &str {
    if file.content_type.trim().is_empty() {
        DEFAULT_CONTENT_TYPE
    } else {
        &file.content_type
    }
}

fn secret_text(
    secrets: &dyn SecretStore,
    dest_id: &str,
    header: &str,
) -> Result<Option<String>, UploadError> {
    match secrets.get(dest_id, &header_secret_field(header)) {
        Ok(Some(bytes)) => String::from_utf8(bytes).map(Some).map_err(|_| {
            UploadError::Permanent("A saved header value is not valid text.".to_owned())
        }),
        Ok(None) => Ok(None),
        Err(_) => Err(UploadError::Permanent(
            "A saved header value could not be read.".to_owned(),
        )),
    }
}

fn expand_link(
    template_text: &str,
    context: &template::TemplateContext<'_>,
) -> Result<String, UploadError> {
    let value = template::expand(template_text, context, false).map_err(template_error)?;
    let value = value.trim();
    if value.is_empty() {
        return Err(UploadError::Permanent(
            "The server did not return a link.".to_owned(),
        ));
    }
    outgoing_url(value)
}

fn expand_optional(
    template_text: Option<&str>,
    context: &template::TemplateContext<'_>,
) -> Result<Option<String>, UploadError> {
    let Some(template_text) = template_text else {
        return Ok(None);
    };
    let value = template::expand(template_text, context, false).map_err(template_error)?;
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        outgoing_url(value).map(Some)
    }
}

fn outgoing_url(value: &str) -> Result<String, UploadError> {
    let value = value.trim();
    match sxcu::url_scheme(value) {
        Some(scheme) if scheme.eq_ignore_ascii_case("https") => Ok(value.to_owned()),
        Some(scheme) if scheme.eq_ignore_ascii_case("http") => Ok(value.to_owned()),
        _ => Err(UploadError::Permanent(
            "The server returned an address that is not https:// or http://.".to_owned(),
        )),
    }
}

fn response_error(
    config: &sxcu::CustomUploaderConfig,
    reply: &HttpReply,
    file_name: &str,
    random: &str,
) -> UploadError {
    let message = config
        .error_message
        .as_deref()
        .and_then(|template_text| {
            let text = reply.text().ok()?;
            let json = serde_json::from_str::<Value>(&text).ok();
            let context = template::TemplateContext {
                file_name,
                random,
                response: Some(&text),
                response_json: json.as_ref(),
            };
            template::expand(template_text, &context, false).ok()
        })
        .map(|message| sanitise_message(message.trim().trim_matches('"')))
        .filter(|message| !message.is_empty());
    match message {
        Some(message) => match reply.status {
            408 | 429 | 500..=599 => UploadError::Retryable(message),
            413 => UploadError::TooLarge,
            401 | 403 => UploadError::Auth,
            _ => UploadError::Permanent(message),
        },
        None => UploadError::from_status_with_retry_after(reply.status, reply.retry_after()),
    }
}

fn sanitise_message(message: &str) -> String {
    let cleaned: String = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    cleaned.trim().chars().take(MAX_ERROR_CHARS).collect()
}

fn template_error(error: template::TemplateError) -> UploadError {
    let message = match error {
        template::TemplateError::InvalidJson => "The server response is not valid JSON.",
        template::TemplateError::InvalidPath => "The JSON path in the uploader is not valid.",
        template::TemplateError::InvalidTemplate => "The uploader template is not valid.",
    };
    UploadError::Permanent(message.to_owned())
}

fn base64_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(BASE64_ALPHABET[(first >> 2) as usize]));
        encoded.push(char::from(
            BASE64_ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize],
        ));
        if chunk.len() > 1 {
            encoded.push(char::from(
                BASE64_ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize],
            ));
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(char::from(BASE64_ALPHABET[(third & 0x3f) as usize]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::sxcu::{BodyType, CustomUploaderConfig, Header};
    use super::*;
    use crate::queue::SecretError;

    struct FakeTransport {
        requests: Mutex<Vec<Request>>,
        replies: Mutex<VecDeque<HttpReply>>,
    }

    impl FakeTransport {
        fn new(replies: Vec<HttpReply>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                replies: Mutex::new(replies.into()),
            }
        }

        fn requests(&self) -> Vec<Request> {
            match self.requests.lock() {
                Ok(requests) => requests.clone(),
                Err(_) => Vec::new(),
            }
        }
    }

    impl Transport for FakeTransport {
        fn execute(&self, request: Request) -> Result<HttpReply, UploadError> {
            if let Ok(mut requests) = self.requests.lock() {
                requests.push(request);
            }
            match self.replies.lock() {
                Ok(mut replies) => replies.pop_front().ok_or_else(|| {
                    UploadError::Permanent("The fake transport has no reply.".to_owned())
                }),
                Err(_) => Err(UploadError::Permanent(
                    "The fake transport is unavailable.".to_owned(),
                )),
            }
        }
    }

    struct FakeSecrets {
        dest_id: String,
        values: Vec<(String, String)>,
    }

    impl FakeSecrets {
        fn with_authorization() -> Self {
            Self {
                dest_id: "custom".to_owned(),
                values: vec![(
                    header_secret_field("Authorization"),
                    "Bearer top-secret".to_owned(),
                )],
            }
        }
    }

    impl SecretStore for FakeSecrets {
        fn get(&self, dest_id: &str, field: &str) -> Result<Option<Vec<u8>>, SecretError> {
            if dest_id != self.dest_id {
                return Ok(None);
            }
            Ok(self
                .values
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, value)| value.as_bytes().to_vec()))
        }
    }

    fn file() -> UploadFile {
        UploadFile::new(
            "C:\\tmp\\shot.png",
            "shot.png",
            "image/png",
            b"hello".to_vec(),
        )
    }

    fn config(body: BodyType) -> CustomUploaderConfig {
        CustomUploaderConfig {
            name: "Example".to_owned(),
            method: Method::Post,
            request_url: "https://example.test/upload".to_owned(),
            body,
            file_form_field: "file".to_owned(),
            link: "{json:$.data.url}".to_owned(),
            thumbnail: Some("{json:$.data.thumb}".to_owned()),
            deletion: Some("{json:$.data.delete}".to_owned()),
            error_message: Some("{json:$.error}".to_owned()),
            headers: vec![Header::new("Authorization", true)],
        }
    }

    fn success() -> HttpReply {
        HttpReply {
            status: 200,
            headers: Vec::new(),
            body: br#"{"data":{"url":"https://i.example/a.png","thumb":"https://i.example/a_t.png","delete":"https://i.example/d/a"}}"#.to_vec(),
        }
    }

    fn uploader(
        config: CustomUploaderConfig,
        replies: Vec<HttpReply>,
    ) -> (CustomUploader, Arc<FakeTransport>) {
        let transport = Arc::new(FakeTransport::new(replies));
        let uploader = CustomUploader::with_transport("custom", config, transport.clone());
        (uploader, transport)
    }

    #[test]
    fn multipart_upload_sends_the_file_and_a_secret_header() {
        let (uploader, transport) = uploader(config(BodyType::MultipartFormData), vec![success()]);
        let uploaded = match uploader.upload_custom(&file(), &FakeSecrets::with_authorization()) {
            Ok(uploaded) => uploaded,
            Err(error) => panic!("upload failed: {error}"),
        };
        assert_eq!(uploaded.uploaded.link, "https://i.example/a.png");
        assert_eq!(
            uploaded.thumbnail_link.as_deref(),
            Some("https://i.example/a_t.png")
        );
        assert_eq!(
            uploaded
                .uploaded
                .deletion_link
                .as_ref()
                .map(|link| link.url.as_str()),
            Some("https://i.example/d/a")
        );
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method(), Method::Post);
        assert_eq!(request.url(), "https://example.test/upload");
        assert!(request
            .headers()
            .iter()
            .any(|(name, value)| { name == "Authorization" && value == "Bearer top-secret" }));
        let content_type = request
            .headers()
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str());
        assert!(content_type.is_some_and(|value| value.starts_with("multipart/form-data")));
        let body = String::from_utf8_lossy(request.body_bytes());
        assert!(body.contains("name=\"file\""));
        assert!(body.contains("filename=\"shot.png\""));
        assert!(body.contains("hello"));
    }

    #[test]
    fn binary_upload_sends_the_raw_bytes() {
        let mut config = config(BodyType::Binary);
        config.method = Method::Put;
        let (uploader, transport) = uploader(config, vec![success()]);
        assert!(uploader
            .upload(&file(), &FakeSecrets::with_authorization())
            .is_ok());
        let requests = transport.requests();
        assert_eq!(requests[0].method(), Method::Put);
        assert_eq!(requests[0].body_bytes(), b"hello");
        assert!(requests[0].headers().iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value == "image/png"
        }));
    }

    #[test]
    fn form_url_encoded_upload_sends_the_base64_field() {
        let mut config = config(BodyType::FormUrlEncoded);
        config.method = Method::Patch;
        let (uploader, transport) = uploader(config, vec![success()]);
        assert!(uploader
            .upload(&file(), &FakeSecrets::with_authorization())
            .is_ok());
        let requests = transport.requests();
        assert_eq!(requests[0].method(), Method::Patch);
        let body = String::from_utf8_lossy(requests[0].body_bytes());
        assert_eq!(body, "file=aGVsbG8%3D");
    }

    #[test]
    fn json_upload_sends_a_json_object() {
        let (uploader, transport) = uploader(config(BodyType::Json), vec![success()]);
        assert!(uploader
            .upload(&file(), &FakeSecrets::with_authorization())
            .is_ok());
        let requests = transport.requests();
        let value: Value = match serde_json::from_slice(requests[0].body_bytes()) {
            Ok(value) => value,
            Err(error) => panic!("the JSON body is invalid: {error}"),
        };
        assert_eq!(value["file"], "aGVsbG8=");
        assert!(requests[0].headers().iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value == "application/json"
        }));
    }

    #[test]
    fn placeholders_expand_in_the_request_url() {
        let mut config = config(BodyType::Binary);
        config.request_url = "https://example.test/{filename}?r={random}".to_owned();
        let transport = Arc::new(FakeTransport::new(vec![success()]));
        let uploader = CustomUploader::with_transport("custom", config, transport.clone());
        assert!(uploader
            .upload(&file(), &FakeSecrets::with_authorization())
            .is_ok());
        let requests = transport.requests();
        let url = requests[0].url();
        let (path, random) = match url.split_once("?r=") {
            Some(parts) => parts,
            None => panic!("the placeholders were not expanded: {url}"),
        };
        assert_eq!(path, "https://example.test/shot.png");
        assert_eq!(random.len(), 4);
        assert!(random
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit()));
    }

    #[test]
    fn a_filename_with_a_space_is_encoded_in_the_url() {
        let mut config = config(BodyType::Binary);
        config.request_url = "https://example.test/{filename}".to_owned();
        let transport = Arc::new(FakeTransport::new(vec![success()]));
        let uploader = CustomUploader::with_transport("custom", config, transport.clone());
        let file = UploadFile::new(
            "C:\\tmp\\shot one.png",
            "shot one.png",
            "image/png",
            b"hello".to_vec(),
        );
        assert!(uploader
            .upload(&file, &FakeSecrets::with_authorization())
            .is_ok());
        assert_eq!(
            transport.requests()[0].url(),
            "https://example.test/shot%20one.png"
        );
    }

    #[test]
    fn the_error_message_is_read_from_the_response_without_headers() {
        let reply = HttpReply {
            status: 400,
            headers: Vec::new(),
            body: br#"{"error":"The file type is not allowed."}"#.to_vec(),
        };
        let (uploader, _) = uploader(config(BodyType::Binary), vec![reply]);
        let error = match uploader.upload(&file(), &FakeSecrets::with_authorization()) {
            Ok(_) => panic!("the upload should have failed"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            UploadError::Permanent("The file type is not allowed.".to_owned())
        );
        assert!(!error.to_string().contains("top-secret"));
    }

    #[test]
    fn a_server_error_message_stays_retryable() {
        let reply = HttpReply {
            status: 503,
            headers: Vec::new(),
            body: br#"{"error":"Try again later."}"#.to_vec(),
        };
        let (uploader, _) = uploader(config(BodyType::Binary), vec![reply]);
        let error = match uploader.upload(&file(), &FakeSecrets::with_authorization()) {
            Ok(_) => panic!("the upload should have failed"),
            Err(error) => error,
        };
        assert_eq!(error, UploadError::Retryable("Try again later.".to_owned()));
    }

    #[test]
    fn statuses_map_like_other_uploaders_when_there_is_no_message() {
        let reply = HttpReply {
            status: 401,
            headers: Vec::new(),
            body: br#"{}"#.to_vec(),
        };
        let (uploader, _) = uploader(config(BodyType::Binary), vec![reply]);
        assert_eq!(
            uploader.upload(&file(), &FakeSecrets::with_authorization()),
            Err(UploadError::Auth)
        );
    }

    #[test]
    fn a_missing_link_is_permanent() {
        let reply = HttpReply {
            status: 200,
            headers: Vec::new(),
            body: br#"{}"#.to_vec(),
        };
        let (uploader, _) = uploader(config(BodyType::Binary), vec![reply]);
        assert_eq!(
            uploader.upload(&file(), &FakeSecrets::with_authorization()),
            Err(UploadError::Permanent(
                "The server did not return a link.".to_owned()
            ))
        );
    }

    #[test]
    fn a_link_with_an_unsafe_scheme_is_rejected() {
        let reply = HttpReply {
            status: 200,
            headers: Vec::new(),
            body: br#"{"data":{"url":"file:///C:/secret.txt"}}"#.to_vec(),
        };
        let (uploader, _) = uploader(config(BodyType::Binary), vec![reply]);
        let error = match uploader.upload(&file(), &FakeSecrets::with_authorization()) {
            Ok(_) => panic!("the upload should have failed"),
            Err(error) => error,
        };
        assert!(matches!(error, UploadError::Permanent(_)));
        assert!(!error.to_string().contains("top-secret"));
    }

    #[test]
    fn the_custom_deletion_link_is_fetched() {
        let reply = HttpReply {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        };
        let (uploader, transport) = uploader(config(BodyType::Binary), vec![reply]);
        let link = DeletionLink::new("custom", "https://i.example/delete/a");
        assert!(uploader
            .delete(&link, &FakeSecrets::with_authorization())
            .is_ok());
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), Method::Get);
        assert_eq!(requests[0].url(), "https://i.example/delete/a");
        assert!(requests[0]
            .headers()
            .iter()
            .any(|(name, value)| { name == "Authorization" && value == "Bearer top-secret" }));
    }

    #[test]
    fn export_reads_header_values_only_when_asked() {
        let (uploader, _) = uploader(config(BodyType::Binary), Vec::new());
        let secrets = FakeSecrets::with_authorization();
        let blank = match uploader.export(false, &secrets) {
            Ok(blank) => blank,
            Err(error) => panic!("export failed: {error}"),
        };
        assert!(!blank.contains("top-secret"));
        let included = match uploader.export(true, &secrets) {
            Ok(included) => included,
            Err(error) => panic!("export failed: {error}"),
        };
        assert!(included.contains("top-secret"));
    }

    #[test]
    fn test_upload_uses_the_same_path() {
        let (uploader, transport) = uploader(config(BodyType::Binary), vec![success()]);
        assert!(uploader
            .test_upload(&file(), &FakeSecrets::with_authorization())
            .is_ok());
        assert_eq!(transport.requests().len(), 1);
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }
}
