use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::http::Method;

use super::template;

pub const MAX_FILE_BYTES: usize = 64 * 1024;
pub const SHAREX_VERSION: &str = "13.7.0";
pub const DEFAULT_FILE_FORM_FIELD: &str = "file";

const DESTINATION_TYPE: &str = "ImageUploader";
const REQUEST_URL_FIELD: &str = "Request URL";
const LINK_FIELD: &str = "Link from response";
const THUMBNAIL_FIELD: &str = "Thumbnail from response";
const DELETION_FIELD: &str = "Deletion link from response";
const ERROR_MESSAGE_FIELD: &str = "Error message from response";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyType {
    MultipartFormData,
    Binary,
    FormUrlEncoded,
    Json,
}

impl BodyType {
    pub const fn as_sharex(self) -> &'static str {
        match self {
            Self::MultipartFormData => "MultipartFormData",
            Self::Binary => "Binary",
            Self::FormUrlEncoded => "FormURLEncoded",
            Self::Json => "JSON",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::MultipartFormData => "Multipart form data",
            Self::Binary => "Binary",
            Self::FormUrlEncoded => "Form URL-encoded",
            Self::Json => "JSON",
        }
    }

    fn from_sharex(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("MultipartFormData") || value.eq_ignore_ascii_case("Multipart")
        {
            Some(Self::MultipartFormData)
        } else if value.eq_ignore_ascii_case("Binary") {
            Some(Self::Binary)
        } else if value.eq_ignore_ascii_case("FormURLEncoded")
            || value.eq_ignore_ascii_case("FormUrlEncoded")
            || value.eq_ignore_ascii_case("FormURL-Encoded")
        {
            Some(Self::FormUrlEncoded)
        } else if value.eq_ignore_ascii_case("JSON") {
            Some(Self::Json)
        } else {
            None
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub secret: bool,
}

impl Header {
    pub fn new(name: impl Into<String>, secret: bool) -> Self {
        Self {
            name: name.into(),
            secret,
        }
    }
}

impl fmt::Debug for Header {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Header")
            .field("name", &self.name)
            .field("secret", &self.secret)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomUploaderConfig {
    pub name: String,
    pub method: Method,
    pub request_url: String,
    pub body: BodyType,
    pub file_form_field: String,
    pub link: String,
    pub thumbnail: Option<String>,
    pub deletion: Option<String>,
    pub error_message: Option<String>,
    pub headers: Vec<Header>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct HeaderValues {
    values: BTreeMap<String, String>,
}

impl HeaderValues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.values.insert(name.into(), value.into());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .map(String::as_str)
            .or_else(|| {
                self.values
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.as_str())
            })
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

impl FromIterator<(String, String)> for HeaderValues {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

impl fmt::Debug for HeaderValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HeaderValues")
            .field("values", &"***")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SxcuWarning {
    InsecureHttp(&'static str),
}

impl SxcuWarning {
    pub fn field(&self) -> &'static str {
        match self {
            Self::InsecureHttp(field) => field,
        }
    }
}

impl fmt::Display for SxcuWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "The {} uses http://, so the connection is not encrypted.",
            self.field()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SxcuError {
    #[error("The .sxcu file is larger than 64 KiB.")]
    TooLarge,
    #[error("The .sxcu file is not valid JSON.")]
    InvalidJson,
    #[error("The request method \"{0}\" is not supported. Use POST, PUT or PATCH.")]
    UnsupportedMethod(String),
    #[error("The body type \"{0}\" is not supported. Use Multipart form data, Binary, Form URL-encoded or JSON.")]
    UnsupportedBody(String),
    #[error("The request URL is missing.")]
    MissingRequestUrl,
    #[error("The file form field is required for multipart uploads.")]
    MissingFileFormField,
    #[error("The {field} must be a full https:// address.")]
    InvalidUrl { field: &'static str },
    #[error("The {field} uses a scheme other than http or https.")]
    UnsupportedScheme { field: &'static str },
    #[error("The {field} contains a placeholder that could not be read.")]
    InvalidTemplate { field: &'static str },
    #[error("The header name \"{0}\" is not valid.")]
    InvalidHeaderName(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSxcu {
    pub config: CustomUploaderConfig,
    pub warnings: Vec<SxcuWarning>,
    pub header_values: HeaderValues,
}

#[derive(Deserialize)]
struct RawSxcu {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "RequestMethod", default)]
    request_method: String,
    #[serde(rename = "RequestURL", default)]
    request_url: String,
    #[serde(rename = "Headers", default)]
    headers: BTreeMap<String, String>,
    #[serde(rename = "Body", default)]
    body: String,
    #[serde(rename = "FileFormName", default)]
    file_form_name: String,
    #[serde(rename = "URL", default)]
    url: String,
    #[serde(rename = "ThumbnailURL", default)]
    thumbnail_url: String,
    #[serde(rename = "DeletionURL", default)]
    deletion_url: String,
    #[serde(rename = "ErrorMessage", default)]
    error_message: String,
}

#[derive(Serialize)]
struct ExportSxcu<'a> {
    #[serde(rename = "Version")]
    version: &'a str,
    #[serde(rename = "Name")]
    name: &'a str,
    #[serde(rename = "DestinationType")]
    destination_type: &'a str,
    #[serde(rename = "RequestMethod")]
    request_method: &'a str,
    #[serde(rename = "RequestURL")]
    request_url: &'a str,
    #[serde(rename = "Headers", skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<&'a str, &'a str>,
    #[serde(rename = "Body")]
    body: &'a str,
    #[serde(rename = "FileFormName", skip_serializing_if = "str::is_empty")]
    file_form_name: &'a str,
    #[serde(rename = "URL")]
    url: &'a str,
    #[serde(rename = "ThumbnailURL", skip_serializing_if = "str::is_empty")]
    thumbnail_url: &'a str,
    #[serde(rename = "DeletionURL", skip_serializing_if = "str::is_empty")]
    deletion_url: &'a str,
    #[serde(rename = "ErrorMessage", skip_serializing_if = "str::is_empty")]
    error_message: &'a str,
}

pub fn import(bytes: &[u8]) -> Result<ImportedSxcu, SxcuError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(SxcuError::TooLarge);
    }
    let raw: RawSxcu = serde_json::from_slice(bytes).map_err(|_| SxcuError::InvalidJson)?;
    from_raw(raw)
}

pub fn import_str(text: &str) -> Result<ImportedSxcu, SxcuError> {
    import(text.as_bytes())
}

pub fn export(
    config: &CustomUploaderConfig,
    include_header_values: bool,
    header_values: &HeaderValues,
) -> Result<String, SxcuError> {
    validate_config(config)?;
    let headers = config
        .headers
        .iter()
        .map(|header| {
            let value = if include_header_values {
                header_values.get(&header.name).unwrap_or("")
            } else {
                ""
            };
            (header.name.as_str(), value)
        })
        .collect();
    let file = ExportSxcu {
        version: SHAREX_VERSION,
        name: &config.name,
        destination_type: DESTINATION_TYPE,
        request_method: config.method.as_str(),
        request_url: &config.request_url,
        headers,
        body: config.body.as_sharex(),
        file_form_name: &config.file_form_field,
        url: &config.link,
        thumbnail_url: config.thumbnail.as_deref().unwrap_or(""),
        deletion_url: config.deletion.as_deref().unwrap_or(""),
        error_message: config.error_message.as_deref().unwrap_or(""),
    };
    serde_json::to_string_pretty(&file).map_err(|_| SxcuError::InvalidJson)
}

pub fn validate_config(config: &CustomUploaderConfig) -> Result<Vec<SxcuWarning>, SxcuError> {
    if !matches!(config.method, Method::Post | Method::Put | Method::Patch) {
        return Err(SxcuError::UnsupportedMethod(
            config.method.as_str().to_owned(),
        ));
    }
    if config.request_url.trim().is_empty() {
        return Err(SxcuError::MissingRequestUrl);
    }
    if config.body == BodyType::MultipartFormData
        && config.file_form_field.trim().is_empty()
    {
        return Err(SxcuError::MissingFileFormField);
    }
    if !valid_header_name(&config.file_form_field) && !config.file_form_field.is_empty() {
        return Err(SxcuError::InvalidHeaderName(
            config.file_form_field.clone(),
        ));
    }
    let mut warnings = Vec::new();
    if let Some(field_warning) =
        validate_url(REQUEST_URL_FIELD, &config.request_url, true, false)?
    {
        warnings.push(field_warning);
    }
    if let Some(field_warning) = validate_url(LINK_FIELD, &config.link, true, true)? {
        warnings.push(field_warning);
    }
    if let Some(thumbnail) = config.thumbnail.as_deref() {
        if let Some(field_warning) = validate_url(THUMBNAIL_FIELD, thumbnail, false, true)? {
            warnings.push(field_warning);
        }
    }
    if let Some(deletion) = config.deletion.as_deref() {
        if let Some(field_warning) = validate_url(DELETION_FIELD, deletion, false, true)? {
            warnings.push(field_warning);
        }
    }
    validate_template(REQUEST_URL_FIELD, &config.request_url)?;
    validate_template(LINK_FIELD, &config.link)?;
    if let Some(thumbnail) = config.thumbnail.as_deref() {
        validate_template(THUMBNAIL_FIELD, thumbnail)?;
    }
    if let Some(deletion) = config.deletion.as_deref() {
        validate_template(DELETION_FIELD, deletion)?;
    }
    if let Some(error_message) = config.error_message.as_deref() {
        validate_template(ERROR_MESSAGE_FIELD, error_message)?;
    }
    for header in &config.headers {
        if !valid_header_name(&header.name) {
            return Err(SxcuError::InvalidHeaderName(header.name.clone()));
        }
    }
    Ok(warnings)
}

pub(crate) fn url_scheme(value: &str) -> Option<&str> {
    let end = value.find(':')?;
    let scheme = &value[..end];
    let mut bytes = scheme.bytes();
    let first = bytes.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')) {
        Some(scheme)
    } else {
        None
    }
}

fn from_raw(raw: RawSxcu) -> Result<ImportedSxcu, SxcuError> {
    let method = method_from_sharex(&raw.request_method)?;
    let body = body_from_sharex(&raw.body)?;
    let request_url = template::migrate_legacy(raw.request_url.trim());
    if request_url.is_empty() {
        return Err(SxcuError::MissingRequestUrl);
    }
    let file_form_field = raw.file_form_name.trim().to_owned();
    let file_form_field = if file_form_field.is_empty() {
        if body == BodyType::MultipartFormData {
            return Err(SxcuError::MissingFileFormField);
        }
        DEFAULT_FILE_FORM_FIELD.to_owned()
    } else {
        file_form_field
    };
    let link = if raw.url.trim().is_empty() {
        "{response}".to_owned()
    } else {
        template::migrate_legacy(raw.url.trim())
    };
    let thumbnail = optional_template(raw.thumbnail_url);
    let deletion = optional_template(raw.deletion_url);
    let error_message = optional_template(raw.error_message);
    let mut headers = Vec::with_capacity(raw.headers.len());
    let mut header_values = HeaderValues::new();
    for (name, value) in raw.headers {
        let name = name.trim().to_owned();
        if !valid_header_name(&name) {
            return Err(SxcuError::InvalidHeaderName(name));
        }
        header_values.insert(name.clone(), value);
        headers.push(Header::new(name, true));
    }
    let name = if raw.name.trim().is_empty() {
        default_name(&request_url)
    } else {
        raw.name.trim().to_owned()
    };
    let config = CustomUploaderConfig {
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
    };
    let warnings = validate_config(&config)?;
    Ok(ImportedSxcu {
        config,
        warnings,
        header_values,
    })
}

fn method_from_sharex(value: &str) -> Result<Method, SxcuError> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("POST") {
        Ok(Method::Post)
    } else if value.eq_ignore_ascii_case("PUT") {
        Ok(Method::Put)
    } else if value.eq_ignore_ascii_case("PATCH") {
        Ok(Method::Patch)
    } else {
        Err(SxcuError::UnsupportedMethod(value.to_owned()))
    }
}

fn body_from_sharex(value: &str) -> Result<BodyType, SxcuError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(BodyType::MultipartFormData);
    }
    BodyType::from_sharex(value).ok_or_else(|| SxcuError::UnsupportedBody(value.to_owned()))
}

fn optional_template(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(template::migrate_legacy(value))
    }
}

fn validate_url(
    field: &'static str,
    value: &str,
    required: bool,
    allow_template: bool,
) -> Result<Option<SxcuWarning>, SxcuError> {
    let value = value.trim();
    if value.is_empty() {
        return if required {
            Err(SxcuError::InvalidUrl { field })
        } else {
            Ok(None)
        };
    }
    if allow_template && (value.starts_with('{') || value.starts_with('$')) {
        return Ok(None);
    }
    match url_scheme(value) {
        Some(scheme) if scheme.eq_ignore_ascii_case("https") => Ok(None),
        Some(scheme) if scheme.eq_ignore_ascii_case("http") => {
            Ok(Some(SxcuWarning::InsecureHttp(field)))
        }
        Some(_) => Err(SxcuError::UnsupportedScheme { field }),
        None => Err(SxcuError::InvalidUrl { field }),
    }
}

fn validate_template(field: &'static str, value: &str) -> Result<(), SxcuError> {
    template::validate(value).map_err(|_| SxcuError::InvalidTemplate { field })
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn default_name(request_url: &str) -> String {
    let host = request_url
        .split_once("://")
        .map_or(request_url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if host.is_empty() {
        "Custom uploader".to_owned()
    } else {
        host.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
        "Version": "13.7.0",
        "Name": "Example host",
        "DestinationType": "ImageUploader",
        "RequestMethod": "POST",
        "RequestURL": "https://example.test/upload",
        "Headers": {
            "Authorization": "Bearer top-secret",
            "X-Client": "snapline"
        },
        "Body": "MultipartFormData",
        "FileFormName": "file",
        "URL": "{json:$.data.url}",
        "ThumbnailURL": "{json:$.data.thumb}",
        "DeletionURL": "{json:$.data.delete}",
        "ErrorMessage": "{json:$.error}"
    }"#;

    fn imported() -> ImportedSxcu {
        match import_str(EXAMPLE) {
            Ok(imported) => imported,
            Err(error) => panic!("the example did not import: {error}"),
        }
    }

    #[test]
    fn sharex_example_round_trips() {
        let first = imported();
        let exported = match export(&first.config, false, &first.header_values) {
            Ok(exported) => exported,
            Err(error) => panic!("the example did not export: {error}"),
        };
        let second = match import_str(&exported) {
            Ok(second) => second,
            Err(error) => panic!("the exported example did not import: {error}"),
        };
        assert_eq!(first.config, second.config);
        assert_eq!(first.warnings, second.warnings);
        assert_eq!(second.config.name, "Example host");
        assert_eq!(second.config.method, Method::Post);
        assert_eq!(second.config.body, BodyType::MultipartFormData);
        assert_eq!(second.config.file_form_field, "file");
        assert_eq!(second.config.link, "{json:$.data.url}");
        assert_eq!(second.config.headers.len(), 2);
    }

    #[test]
    fn export_blanks_header_values_unless_requested() {
        let imported = imported();
        let blank = export(&imported.config, false, &imported.header_values);
        assert!(blank.is_ok());
        let blank = match blank {
            Ok(blank) => blank,
            Err(error) => panic!("export failed: {error}"),
        };
        assert!(!blank.contains("top-secret"));
        assert!(!blank.contains("snapline\""));
        let parsed: serde_json::Value = match serde_json::from_str(&blank) {
            Ok(parsed) => parsed,
            Err(error) => panic!("exported JSON is invalid: {error}"),
        };
        assert_eq!(parsed["Headers"]["Authorization"], "");
        assert_eq!(parsed["Headers"]["X-Client"], "");

        let included = export(&imported.config, true, &imported.header_values);
        let included = match included {
            Ok(included) => included,
            Err(error) => panic!("export failed: {error}"),
        };
        assert!(included.contains("top-secret"));
    }

    #[test]
    fn header_values_are_redacted_in_debug() {
        let imported = imported();
        let debug = format!("{:?}", imported);
        assert!(!debug.contains("top-secret"));
        assert!(debug.contains("Authorization"));
    }

    #[test]
    fn files_larger_than_64_kib_are_rejected() {
        let mut bytes = vec![b' '; MAX_FILE_BYTES + 1];
        bytes[0] = b'{';
        bytes[1] = b'}';
        assert_eq!(import(&bytes), Err(SxcuError::TooLarge));
    }

    #[test]
    fn a_file_at_the_limit_is_read() {
        let mut bytes = b"{}".to_vec();
        bytes.resize(MAX_FILE_BYTES, b' ');
        assert!(matches!(import(&bytes), Err(SxcuError::MissingRequestUrl)));
    }

    #[test]
    fn http_is_a_warning_and_https_is_not() {
        let secure = import_str(r#"{"Name":"A","RequestURL":"https://example.test/upload","Body":"MultipartFormData","FileFormName":"file","URL":"https://example.test/{json:$.url}"}"#);
        let secure = match secure {
            Ok(secure) => secure,
            Err(error) => panic!("secure example failed: {error}"),
        };
        assert!(secure.warnings.is_empty());
        let insecure = import_str(r#"{"Name":"A","RequestURL":"http://example.test/upload","Body":"Binary","URL":"{response}"}"#);
        let insecure = match insecure {
            Ok(insecure) => insecure,
            Err(error) => panic!("insecure example failed: {error}"),
        };
        assert_eq!(insecure.warnings.len(), 1);
        assert_eq!(insecure.warnings[0].field(), REQUEST_URL_FIELD);
    }

    #[test]
    fn file_and_other_schemes_are_rejected() {
        let file = import_str(r#"{"Name":"A","RequestURL":"file:///C:/secret.txt","Body":"Binary","URL":"{response}"}"#);
        assert!(matches!(file, Err(SxcuError::UnsupportedScheme { .. })));
        let link = import_str(r#"{"Name":"A","RequestURL":"https://example.test/upload","Body":"Binary","URL":"ftp://example.test/file"}"#);
        assert!(matches!(link, Err(SxcuError::UnsupportedScheme { .. })));
        let relative = import_str(r#"{"Name":"A","RequestURL":"example.test/upload","Body":"Binary","URL":"{response}"}"#);
        assert!(matches!(relative, Err(SxcuError::InvalidUrl { .. })));
    }

    #[test]
    fn unsupported_methods_and_bodies_are_rejected() {
        let method = import_str(r#"{"Name":"A","RequestMethod":"DELETE","RequestURL":"https://example.test/upload","Body":"Binary","URL":"{response}"}"#);
        assert!(matches!(method, Err(SxcuError::UnsupportedMethod(_))));
        let body = import_str(r#"{"Name":"A","RequestMethod":"POST","RequestURL":"https://example.test/upload","Body":"XML","URL":"{response}"}"#);
        assert!(matches!(body, Err(SxcuError::UnsupportedBody(_))));
    }

    #[test]
    fn every_body_type_is_accepted() {
        for (sharex, expected) in [
            ("MultipartFormData", BodyType::MultipartFormData),
            ("Binary", BodyType::Binary),
            ("FormURLEncoded", BodyType::FormUrlEncoded),
            ("JSON", BodyType::Json),
        ] {
            let json = format!(
                r#"{{"Name":"A","RequestURL":"https://example.test/upload","Body":"{sharex}","FileFormName":"file","URL":"{{response}}"}}"#
            );
            let imported = match import_str(&json) {
                Ok(imported) => imported,
                Err(error) => panic!("{sharex} was rejected: {error}"),
            };
            assert_eq!(imported.config.body, expected);
            assert_eq!(imported.config.body.as_sharex(), sharex);
        }
    }

    #[test]
    fn a_missing_request_url_is_rejected() {
        assert!(matches!(
            import_str(r#"{"Name":"A","Body":"Binary"}"#),
            Err(SxcuError::MissingRequestUrl)
        ));
    }

    #[test]
    fn a_missing_multipart_file_field_is_rejected() {
        assert!(matches!(
            import_str(r#"{"Name":"A","RequestURL":"https://example.test/upload","Body":"MultipartFormData","URL":"{response}"}"#),
            Err(SxcuError::MissingFileFormField)
        ));
    }

    #[test]
    fn invalid_header_names_are_rejected() {
        assert!(matches!(
            import_str(r#"{"Name":"A","RequestURL":"https://example.test/upload","Body":"Binary","Headers":{"Bad Header":"x"},"URL":"{response}"}"#),
            Err(SxcuError::InvalidHeaderName(_))
        ));
    }

    #[test]
    fn legacy_url_fields_are_migrated() {
        let imported = import_str(r#"{"Name":"A","RequestURL":"https://example.test/$random$","Body":"Binary","URL":"$json:data.url$"}"#);
        let imported = match imported {
            Ok(imported) => imported,
            Err(error) => panic!("legacy example failed: {error}"),
        };
        assert_eq!(imported.config.request_url, "https://example.test/{random}");
        assert_eq!(imported.config.link, "{json:data.url}");
    }

    #[test]
    fn invalid_json_paths_are_rejected() {
        assert!(matches!(
            import_str(r#"{"Name":"A","RequestURL":"https://example.test/upload","Body":"Binary","URL":"{json:}"}"#),
            Err(SxcuError::InvalidTemplate { .. })
        ));
    }

    #[test]
    fn the_name_falls_back_to_the_host() {
        let imported = import_str(r#"{"RequestURL":"https://upload.example.test:8443/put","Body":"Binary","URL":"{response}"}"#);
        let imported = match imported {
            Ok(imported) => imported,
            Err(error) => panic!("nameless example failed: {error}"),
        };
        assert_eq!(imported.config.name, "upload.example.test:8443");
    }

    #[test]
    fn the_link_falls_back_to_the_response() {
        let imported = import_str(r#"{"Name":"A","RequestURL":"https://example.test/upload","Body":"Binary"}"#);
        let imported = match imported {
            Ok(imported) => imported,
            Err(error) => panic!("linkless example failed: {error}"),
        };
        assert_eq!(imported.config.link, "{response}");
    }

    #[test]
    fn labels_match_the_settings_form() {
        assert_eq!(BodyType::MultipartFormData.label(), "Multipart form data");
        assert_eq!(BodyType::Binary.label(), "Binary");
        assert_eq!(BodyType::FormUrlEncoded.label(), "Form URL-encoded");
        assert_eq!(BodyType::Json.label(), "JSON");
    }
}
