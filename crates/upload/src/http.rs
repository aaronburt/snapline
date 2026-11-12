use std::io::Read;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Proxy};

use crate::error::UploadError;

pub const USER_AGENT: &str = concat!("Snapline/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
        }
    }

    fn to_ureq(self) -> ureq::http::Method {
        match self {
            Self::Get => ureq::http::Method::GET,
            Self::Post => ureq::http::Method::POST,
            Self::Put => ureq::http::Method::PUT,
            Self::Patch => ureq::http::Method::PATCH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpTimeout {
    ThirtySeconds,
    SixtySeconds,
    OneHundredTwentySeconds,
}

impl HttpTimeout {
    pub const fn seconds(self) -> u64 {
        match self {
            Self::ThirtySeconds => 30,
            Self::SixtySeconds => 60,
            Self::OneHundredTwentySeconds => 120,
        }
    }
}

const SLOWEST_UPLOAD_BYTES_PER_SECOND: u64 = 32 * 1024;
const SEND_BODY_ALLOWANCE: Duration =
    Duration::from_secs(crate::queue::MAX_BUFFERED_UPLOAD_BYTES / SLOWEST_UPLOAD_BYTES_PER_SECOND);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxySetting {
    System,
    None,
    Custom(String),
}

impl ProxySetting {
    pub fn system(url: Option<String>) -> Self {
        url.map_or(Self::None, Self::Custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    pub fn put(url: impl Into<String>) -> Self {
        Self::new(Method::Put, url)
    }

    pub fn patch(url: impl Into<String>) -> Self {
        Self::new(Method::Patch, url)
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn body_bytes(&self) -> &[u8] {
        &self.body
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set_header(name.into(), value.into());
        self
    }

    pub fn body(mut self, content_type: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.set_header("Content-Type".to_owned(), content_type.to_owned());
        self.body = bytes.into();
        self
    }

    pub fn form(self, fields: &[(&str, &str)]) -> Self {
        let mut encoded = String::new();
        for (index, (name, value)) in fields.iter().enumerate() {
            if index > 0 {
                encoded.push('&');
            }
            encoded.push_str(&encode_form_component(name));
            encoded.push('=');
            encoded.push_str(&encode_form_component(value));
        }
        self.body("application/x-www-form-urlencoded", encoded.into_bytes())
    }

    pub fn json<T: Serialize>(self, value: &T) -> Result<Self, UploadError> {
        let bytes = serde_json::to_vec(value).map_err(|error| UploadError::from_json(&error))?;
        Ok(self.body("application/json", bytes))
    }

    pub fn multipart(self, form: Multipart) -> Self {
        let (content_type, bytes) = form.into_encoded();
        self.body(&content_type, bytes)
    }

    fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn set_header(&mut self, name: String, value: String) {
        self.headers
            .retain(|(existing, _)| !existing.eq_ignore_ascii_case(&name));
        self.headers.push((name, value));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multipart {
    parts: Vec<MultipartPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MultipartPart {
    Text {
        name: String,
        value: String,
    },
    File {
        name: String,
        file_name: String,
        content_type: String,
        bytes: Vec<u8>,
    },
}

impl Multipart {
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parts.push(MultipartPart::Text {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    pub fn file(
        mut self,
        name: impl Into<String>,
        file_name: impl Into<String>,
        content_type: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        self.parts.push(MultipartPart::File {
            name: name.into(),
            file_name: file_name.into(),
            content_type: content_type.into(),
            bytes: bytes.into(),
        });
        self
    }

    fn into_encoded(self) -> (String, Vec<u8>) {
        const BOUNDARY: &str = "----SnaplineFormBoundaryJ7x2Vq9mR4tL";
        let capacity = self
            .parts
            .iter()
            .map(|part| match part {
                MultipartPart::Text { name, value } => name.len() + value.len() + 128,
                MultipartPart::File {
                    name,
                    file_name,
                    content_type,
                    bytes,
                } => name.len() + file_name.len() + content_type.len() + bytes.len() + 192,
            })
            .sum::<usize>()
            + BOUNDARY.len()
            + 8;
        let mut body = Vec::with_capacity(capacity);
        // Each part is dropped as soon as it is copied, so its bytes are not held twice for long.
        for part in self.parts {
            body.extend_from_slice(b"--");
            body.extend_from_slice(BOUNDARY.as_bytes());
            body.extend_from_slice(b"\r\n");
            match part {
                MultipartPart::Text { name, value } => {
                    let header = format!(
                        "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                        escape_field(&name)
                    );
                    body.extend_from_slice(header.as_bytes());
                    body.extend_from_slice(value.as_bytes());
                }
                MultipartPart::File {
                    name,
                    file_name,
                    content_type,
                    bytes,
                } => {
                    let header = format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                        escape_field(&name),
                        escape_field(&file_name),
                        escape_field(&content_type)
                    );
                    body.extend_from_slice(header.as_bytes());
                    body.extend_from_slice(&bytes);
                }
            }
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={BOUNDARY}"), body)
    }
}

impl Default for Multipart {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn text(&self) -> Result<String, UploadError> {
        String::from_utf8(self.body.clone()).map_err(|_| {
            UploadError::Permanent("The server response is not valid UTF-8.".to_owned())
        })
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, UploadError> {
        serde_json::from_slice(&self.body).map_err(|error| UploadError::from_json(&error))
    }

    pub fn retry_after(&self) -> Option<Duration> {
        self.header("retry-after")?
            .trim()
            .parse::<u64>()
            .ok()
            .map(Duration::from_secs)
    }

    pub fn is_success(&self) -> bool {
        (200..=299).contains(&self.status)
    }

    pub fn error_for_status(&self) -> Result<(), UploadError> {
        if self.is_success() {
            return Ok(());
        }
        Err(UploadError::from_status_with_retry_after(
            self.status,
            self.retry_after(),
        ))
    }
}

pub trait HttpTransport: Send + Sync {
    fn execute(&self, request: Request) -> Result<Response, UploadError>;

    fn send(&self, request: Request) -> Result<Response, UploadError> {
        let response = self.execute(request)?;
        response.error_for_status()?;
        Ok(response)
    }

    fn get(&self, url: &str) -> Result<Response, UploadError> {
        self.send(Request::get(url))
    }

    fn post_bytes(
        &self,
        url: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<Response, UploadError> {
        self.send(Request::post(url).body(content_type, bytes.to_vec()))
    }

    fn put_bytes(
        &self,
        url: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<Response, UploadError> {
        self.send(Request::put(url).body(content_type, bytes.to_vec()))
    }

    fn patch_bytes(
        &self,
        url: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<Response, UploadError> {
        self.send(Request::patch(url).body(content_type, bytes.to_vec()))
    }

    fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> Result<Response, UploadError> {
        self.send(Request::post(url).form(fields))
    }

    fn post_multipart(&self, url: &str, form: Multipart) -> Result<Response, UploadError> {
        self.send(Request::post(url).multipart(form))
    }
}

pub struct HttpClient {
    agent: Agent,
}

impl HttpClient {
    pub fn new(timeout: HttpTimeout, proxy: ProxySetting) -> Result<Self, UploadError> {
        Self::build(Duration::from_secs(timeout.seconds()), proxy)
    }

    fn build(timeout: Duration, proxy: ProxySetting) -> Result<Self, UploadError> {
        Self::build_with_tls(timeout, proxy, production_tls())
    }

    fn build_with_tls(
        timeout: Duration,
        proxy: ProxySetting,
        tls: TlsConfig,
    ) -> Result<Self, UploadError> {
        let proxy = match proxy {
            ProxySetting::System => Proxy::try_from_env(),
            ProxySetting::None => None,
            ProxySetting::Custom(url) => Some(Proxy::new(&url).map_err(|_| {
                UploadError::Permanent("The proxy address is not valid.".to_owned())
            })?),
        };
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .user_agent(USER_AGENT)
            .timeout_resolve(Some(timeout))
            .timeout_connect(Some(timeout))
            .timeout_send_request(Some(timeout))
            .timeout_send_body(Some(timeout + SEND_BODY_ALLOWANCE))
            .timeout_recv_response(Some(timeout))
            .timeout_recv_body(Some(timeout))
            .tls_config(tls)
            .proxy(proxy)
            .build();
        Ok(Self {
            agent: Agent::new_with_config(config),
        })
    }

    #[cfg(test)]
    fn with_duration(timeout: Duration) -> Self {
        Self::build(timeout, ProxySetting::None).expect("test client")
    }

    #[cfg(test)]
    fn with_tls(timeout: Duration, tls: TlsConfig) -> Self {
        Self::build_with_tls(timeout, ProxySetting::None, tls).expect("test client")
    }
}

impl HttpTransport for HttpClient {
    fn execute(&self, mut request: Request) -> Result<Response, UploadError> {
        let body = std::mem::take(&mut request.body);
        let mut builder = ureq::http::Request::builder()
            .method(request.method().to_ureq())
            .uri(request.url());
        for (name, value) in request.headers() {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let built = builder
            .body(body)
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
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut body)
            .map_err(|error| UploadError::from_io(&error))?;
        if body.len() > 8 * 1024 * 1024 {
            return Err(UploadError::TooLarge);
        }
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

fn production_tls() -> TlsConfig {
    TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::PlatformVerifier)
        .build()
}

#[cfg(test)]
fn tls_with_roots(roots: RootCerts) -> TlsConfig {
    TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(roots)
        .build()
}

#[cfg(test)]
fn fixture_ca_roots() -> RootCerts {
    use ureq::tls::Certificate;
    const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIDJzCCAg+gAwIBAgIUcKHpCPJkrxgX1qa8TUQFDQCUoj8wDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQU25hcGxpbmUgVGVzdCBDQTAeFw0yNjExMDkwODQ5MTVa
Fw0zMTExMDgwODQ5MTVaMBsxGTAXBgNVBAMMEFNuYXBsaW5lIFRlc3QgQ0EwggEi
MA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDCWZbfW9UPtOxufDMzUBhZgMZ1
QVPcmHQB2ACAvKrB+GnVbrFVU/P81XV24seq/jZgKEYfn3EfWzzKO2nSHscAEn5A
okyA+3elN1O5Q/uWEoTk5T7FOfBfYpMKOlW5YMuv+nRKWp4VJUHFGjfbnmje1Ly+
KSRTjp5NEhFHjALIx+AM8ZDMmADEGsSAuWq1qDrK3T8os9c4Lk8qNy+lNyaCYQj5
bT4Ty8dBhv2h3ehxtnRDe9qwyqMHQkTS7qYPKEXfgGq1hcchMn5JYXzJ4oXkDW/P
3lujijo7AkdW10+ub6YcOdoW+/hP5DY3NyoX7PGVEyTR6a716Z3gb4HLWKsjAgMB
AAGjYzBhMB0GA1UdDgQWBBSARj1SlQgHVOMd++1cog7TsuIXxTAfBgNVHSMEGDAW
gBSARj1SlQgHVOMd++1cog7TsuIXxTAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB
/wQEAwIBBjANBgkqhkiG9w0BAQsFAAOCAQEAiCL9Cy1jvWixLnnchV733kFPB8op
BXCHDM7YaOH6G2ApjhuLkQOKh49pzkYytBbMpS1NbNhu0lnOMAIspDJSCKZuFNpK
ukUzGR0pm2XGbsrPacTElz1TGXiQiLSpYdGEqpFEzv+o68QsrZzqos2uE+65P6we
0XOZlahvQKJBoBAO+OsxTaLWTqI6iuZ5yWuu+wr+2a8ZOEeCgFU2pzIL/y/2ukiR
7jYqgAiaQUwtPbZO5I9Qo8PpGCWOPWlzy39WfrGy6N5UyfDAaLgYnEAh56D/apPY
3gWUC2GZnluEnslr2d6CQnsC8cgEP5rxv0FOuaWl7yKQiJTrlD7XFhU0Zg==
-----END CERTIFICATE-----";
    let certificate = Certificate::from_pem(TEST_CA_PEM.as_bytes()).expect("fixture CA");
    RootCerts::new_with_certs(&[certificate])
}

fn encode_form_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn escape_field(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() && *character != '"' && *character != '\\')
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::sign::CertifiedKey;
    use tiny_http::{Header, Request as StubRequest, Response as StubResponse, Server};

    use super::*;

    const LEAF_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIDUDCCAjigAwIBAgIUTrZzMw27ho7Vhxi3nmqoDhYeIuIwDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQU25hcGxpbmUgVGVzdCBDQTAeFw0yNjExMDkwODUxMjVa
Fw0zNjExMDYwODUxMjVaMBQxEjAQBgNVBAMMCWxvY2FsaG9zdDCCASIwDQYJKoZI
hvcNAQEBBQADggEPADCCAQoCggEBAKJ2Qo8Cc9ooNp3sMsQSFIY9ePqbNLVv5jsv
3sChcjh373PpLU0T2Iaxgyj2bw1+x2HrEMQdwh4ruADlXHlwn5CbGet8TFXAFM/o
e6yZHLARdo06TlgsixAqVJLw4xjaug43Se/mmS/4C+JRHpBl0tlb/KK5MgWPOCzI
fq/UCowIWi1S1h+O54j5SKKPGA8GPlPF70kc9YMbG4blrz1mknCmxqxrdsb8uJAW
2Pzt8w7m84DyC9uZYqKq3jnXYZxFjng52kKvUvGR9sHrwciEP/tgGE+NOPuNtgBh
7gwI/o5RsGqmbxPR724pRWfXdurjvA2FL2xVUHBLx5t48jUozQECAwEAAaOBkjCB
jzAMBgNVHRMBAf8EAjAAMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEF
BQcDATAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwHQYDVR0OBBYEFHg+lNfZ
HrIJQ3mToYjqAO7mFebdMB8GA1UdIwQYMBaAFIBGPVKVCAdU4x377VyiDtOy4hfF
MA0GCSqGSIb3DQEBCwUAA4IBAQCbxiVjZeOTdvsIL0Y/n8rO2BSgYtcom65OOGOK
tlpwMDdTmhaq9MDOPY+z+G130F06DJmPu6mzN13/aAhJICSjtVq++kN0y58GtwqQ
214cliTWkdFvD9TgoR+7qkKdOiKKvF8D+Z1BjJ/Nyhc3i2ObQtSLTyFfWHzdUotj
R7pj2uSubTZpvXejrNEairymt3srat2exHoT/i51QzMFypoAS/yQihJDLsedjGNx
7zILalgvb0I0beyy/MwRbVjZ+AIs0P2RQvM3kZnRudKurIDWKeKaUNx1vUWjArrQ
SKwSY4YHvIhcWWgT6QyrW7tBsL+XLHLdobZQeV0NfWZvugAN
-----END CERTIFICATE-----";

    const LEAF_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCidkKPAnPaKDad
7DLEEhSGPXj6mzS1b+Y7L97AoXI4d+9z6S1NE9iGsYMo9m8Nfsdh6xDEHcIeK7gA
5Vx5cJ+QmxnrfExVwBTP6HusmRywEXaNOk5YLIsQKlSS8OMY2roON0nv5pkv+Avi
UR6QZdLZW/yiuTIFjzgsyH6v1AqMCFotUtYfjueI+UiijxgPBj5Txe9JHPWDGxuG
5a89ZpJwpsasa3bG/LiQFtj87fMO5vOA8gvbmWKiqt4512GcRY54OdpCr1LxkfbB
68HIhD/7YBhPjTj7jbYAYe4MCP6OUbBqpm8T0e9uKUVn13bq47wNhS9sVVBwS8eb
ePI1KM0BAgMBAAECggEAIfSIpb4z+5EzFR+0XpGO053OhsZgpllMa+tt6aH0rwO2
Ouot6zGuLDy2tp2HCXGJd5g7SVoU49xsWVeW09TrrLwWF0dD5xo931RuYN0hh1Un
C453qGsq54aJZarONDgm8eixXeA6/TEhPryIoFJ/1qkorNP7NflAmu+Otu4hBDI3
JAgdy/peD9wZVRij12pQMdR7q4G8Hl2YbZDXZY/wP136WoWqaQxNwITAQOASKbcC
2Tv73FkjLig1p55tHon4QteSrR0dSatxDQ3M9fXwGt53Qy48k/iKd67xmLIiMfDx
2ZQ7sK10kydhaShg7VXDB2BRSU4C/xh089WJd1BNvwKBgQDeDpdDCZXYITvHn1Pf
xlyjkWLfhelwRJwR99iuERZPlmYIw8FoOWvjLV4C7+OXVqsP2OCx4uzI8sEWvz3r
YB9enxoXa7g8EvF8EA7po60plhav/EWepkFa9rcrj+gQJa3USJ9OYzbAZQc9vaHu
mjelOUcbUQhXFyq2hApdWPEKkwKBgQC7S6Eo+8koe3uVFN/w+Q0wkAsZaBBAP/M0
v0++4hKIe+wtR6KEUjaOtIHkpSYJajh1ZJoGuza0yA/ozjlr01YoSErlbjk7xE7G
GQqDDgzYwtrIG0IAIZnMAKRChjnde+WkQpVeZ4cA75XNFPCOoNGcaPVC1AXFRIsj
a6gVZnzCmwKBgC1ctbHQJ3g7ccbZ2XyBPTN848HdzU5N9cQF6ZHFk9U2WCm6M6xi
7pM+FVJsRwYHgnJ+T4iHXu++B4vqDXAj9sSRo/bc3ooHJKW/cgxwj1NLgd75ng9G
z1sRw2F2yGVkDRqsqz6MLgcnnE8UihwADRYHmWsPJrED3onid2uSW8tZAoGAfdB4
GgfJQSByC9tR85XZfl3ElvSItiA/R5ecAiXrPTPBm0kbqrdvXr1y5BzsSDbKZlGG
Bi+MuDQMO98MQ5pbrgOiDCzW1YcxQ3Fe1lqB1rpOLU3iMCRiI9/qd6WjCV1JDeaM
YDY7Qi2u+2yti6IMLH11A8dZeQJqVQjkiDuoYzECgYEAsyhfBOJLiUbq90uDHtui
h6L0cz2wzT9SWHez7BUSpspZRNIIua/42xWbC+vt2ikPkYJ1fMyZWCSLRtqCJjMz
ZKSLu2FxCStCRI5UYHyWh4Tg4LXE4UccieWWYzhFc1NhBUjnO3sLBBDgV9w0KLxl
c4oPTMupbl8Tg8oYB46V5Cg=
-----END PRIVATE KEY-----";

    #[derive(Debug)]
    struct OneCert(Arc<CertifiedKey>);

    impl rustls::server::ResolvesServerCert for OneCert {
        fn resolve(
            &self,
            _client_hello: rustls::server::ClientHello<'_>,
        ) -> Option<Arc<CertifiedKey>> {
            Some(Arc::clone(&self.0))
        }
    }

    fn tls_server() -> (u16, std::thread::JoinHandle<()>) {
        let provider = rustls::crypto::ring::default_provider();
        let _ = provider.clone().install_default();
        let certificate = CertifiedKey::from_der(
            vec![CertificateDer::from(pem_der(LEAF_CERT_PEM))],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pem_der(LEAF_KEY_PEM))),
            &provider,
        )
        .expect("server certificate");
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(OneCert(Arc::new(certificate))));
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let port = listener.local_addr().expect("local address").port();
        let handle = std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let Ok(connection) = rustls::ServerConnection::new(Arc::new(config)) else {
                return;
            };
            let mut stream = rustls::StreamOwned::new(connection, stream);
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        request.extend_from_slice(&buffer[..read]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            let _ = stream.flush();
        });
        (port, handle)
    }

    fn pem_der(pem: &str) -> Vec<u8> {
        let payload: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        decode_base64(&payload)
    }

    fn decode_base64(input: &str) -> Vec<u8> {
        let mut output = Vec::new();
        let mut buffer = 0_u32;
        let mut bits = 0_u32;
        for character in input.bytes() {
            let value = match character {
                b'A'..=b'Z' => u32::from(character - b'A'),
                b'a'..=b'z' => u32::from(character - b'a') + 26,
                b'0'..=b'9' => u32::from(character - b'0') + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                _ => continue,
            };
            buffer = (buffer << 6) | value;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                output.push(u8::try_from((buffer >> bits) & 0xff).unwrap_or(0));
                buffer &= (1_u32 << bits) - 1;
            }
        }
        output
    }

    fn serve(
        handler: impl FnOnce(&mut StubRequest) -> StubResponse<Cursor<Vec<u8>>> + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let server = Server::http("127.0.0.1:0").expect("stub server");
        let url = format!("http://{}", server.server_addr());
        let handle = std::thread::spawn(move || {
            if let Ok(mut request) = server.recv() {
                let response = handler(&mut request);
                let _ = request.respond(response);
            }
        });
        (url, handle)
    }

    fn header(request: &StubRequest, name: &'static str) -> String {
        request
            .headers()
            .iter()
            .find(|header| header.field.equiv(name))
            .map(|header| header.value.as_str().to_owned())
            .unwrap_or_default()
    }

    fn read_body(request: &mut StubRequest) -> String {
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);
        body
    }

    #[test]
    fn get_returns_status_headers_and_body() {
        let (url, handle) = serve(|_| {
            StubResponse::from_string("hello")
                .with_status_code(201)
                .with_header(Header::from_bytes("X-Test", "yes").expect("header"))
        });
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let response = client.get(&url).expect("response");
        handle.join().expect("server");
        assert_eq!(response.status(), 201);
        assert_eq!(response.header("x-test"), Some("yes"));
        assert_eq!(response.text().expect("text"), "hello");
    }

    #[test]
    fn user_agent_and_custom_headers_are_sent() {
        let (url, handle) = serve(|request| {
            StubResponse::from_string(format!(
                "{}|{}",
                header(request, "User-Agent"),
                header(request, "X-Custom")
            ))
        });
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let response = client
            .send(Request::get(&url).header("X-Custom", "value"))
            .expect("response");
        handle.join().expect("server");
        assert_eq!(
            response.text().expect("text"),
            format!("{USER_AGENT}|value")
        );
    }

    #[test]
    fn not_found_is_permanent() {
        let (url, handle) = serve(|_| StubResponse::from_string("missing").with_status_code(404));
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let error = client.get(&url).expect_err("404");
        handle.join().expect("server");
        assert!(matches!(error, UploadError::Permanent(_)));
    }

    #[test]
    fn too_many_requests_reports_retry_after() {
        let (url, handle) = serve(|_| {
            StubResponse::from_string("slow down")
                .with_status_code(429)
                .with_header(Header::from_bytes("Retry-After", "2").expect("header"))
        });
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let response = client.execute(Request::get(&url)).expect("response");
        assert_eq!(response.retry_after(), Some(Duration::from_secs(2)));
        let error = response.error_for_status().expect_err("429");
        handle.join().expect("server");
        assert_eq!(
            error,
            UploadError::Retryable("The server returned HTTP 429, retry after 2 s.".to_owned())
        );
    }

    #[test]
    fn server_error_is_retryable() {
        let (url, handle) =
            serve(|_| StubResponse::from_string("unavailable").with_status_code(503));
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let error = client.get(&url).expect_err("503");
        handle.join().expect("server");
        assert!(error.is_retryable());
    }

    #[test]
    fn slow_server_times_out() {
        let (url, handle) = serve(|_| {
            std::thread::sleep(Duration::from_millis(300));
            StubResponse::from_string("late")
        });
        let client = HttpClient::with_duration(Duration::from_millis(50));
        let error = client.get(&url).expect_err("timeout");
        handle.join().expect("server");
        assert_eq!(
            error,
            UploadError::Retryable("The request timed out.".to_owned())
        );
    }

    #[test]
    fn a_body_that_takes_longer_than_the_timeout_to_send_still_uploads() {
        const BODY_BYTES: usize = 32 * 1024 * 1024;
        let (url, handle) = serve(|request| {
            std::thread::sleep(Duration::from_millis(400));
            let mut body = Vec::new();
            let _ = request.as_reader().read_to_end(&mut body);
            StubResponse::from_string(body.len().to_string())
        });
        let client = HttpClient::with_duration(Duration::from_millis(150));
        let response = client
            .send(Request::post(&url).body("application/octet-stream", vec![7_u8; BODY_BYTES]))
            .expect("slow upload");
        handle.join().expect("server");
        assert_eq!(response.text().expect("text"), BODY_BYTES.to_string());
    }

    #[test]
    fn the_send_allowance_covers_the_largest_buffered_upload_on_a_slow_link() {
        assert_eq!(SEND_BODY_ALLOWANCE, Duration::from_secs(2048));
    }

    #[test]
    fn form_body_is_url_encoded() {
        let (url, handle) = serve(|request| {
            let content_type = header(request, "Content-Type");
            StubResponse::from_string(format!("{content_type}|{}", read_body(request)))
        });
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let response = client
            .post_form(&url, &[("a", "hello world"), ("b", "/x")])
            .expect("response");
        handle.join().expect("server");
        assert_eq!(
            response.text().expect("text"),
            "application/x-www-form-urlencoded|a=hello+world&b=%2Fx"
        );
    }

    #[test]
    fn json_body_sets_the_content_type() {
        let (url, handle) = serve(|request| {
            let content_type = header(request, "Content-Type");
            StubResponse::from_string(format!("{content_type}|{}", read_body(request)))
        });
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let request = Request::post(&url)
            .json(&serde_json::json!({"name": "snapline"}))
            .expect("json");
        let response = client.send(request).expect("response");
        handle.join().expect("server");
        assert_eq!(
            response.text().expect("text"),
            "application/json|{\"name\":\"snapline\"}"
        );
    }

    #[test]
    fn multipart_body_has_a_boundary_and_parts() {
        let (url, handle) = serve(|request| {
            let content_type = header(request, "Content-Type");
            StubResponse::from_string(format!("{content_type}|{}", read_body(request)))
        });
        let form = Multipart::new().text("name", "snapline").file(
            "file",
            "shot.png",
            "image/png",
            vec![1_u8, 2, 3],
        );
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let response = client.post_multipart(&url, form).expect("response");
        handle.join().expect("server");
        let text = response.text().expect("text");
        assert!(text.contains("multipart/form-data; boundary=----SnaplineFormBoundary"));
        assert!(text.contains("name=\"name\"\r\n\r\nsnapline"));
        assert!(text.contains("filename=\"shot.png\"\r\nContent-Type: image/png"));
        assert!(text.contains("--\r\n"));
    }

    #[test]
    fn put_and_patch_use_their_methods() {
        for (method, expected) in [(Method::Put, "PUT"), (Method::Patch, "PATCH")] {
            let (url, handle) =
                serve(|request| StubResponse::from_string(request.method().to_string()));
            let client = HttpClient::with_duration(Duration::from_secs(5));
            let response = client
                .send(Request::new(method, url.as_str()))
                .expect("response");
            handle.join().expect("server");
            assert_eq!(response.text().expect("text"), expected);
        }
    }

    struct FakeClient {
        requests: Arc<Mutex<Vec<Request>>>,
        response: Response,
    }

    impl HttpTransport for FakeClient {
        fn execute(&self, request: Request) -> Result<Response, UploadError> {
            self.requests.lock().expect("lock").push(request);
            Ok(self.response.clone())
        }
    }

    #[test]
    fn a_fake_transport_supports_the_helpers() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = FakeClient {
            requests: requests.clone(),
            response: Response {
                status: 200,
                headers: Vec::new(),
                body: b"ok".to_vec(),
            },
        };
        let response = client
            .post_bytes("https://example.test/upload", "image/png", &[1, 2, 3])
            .expect("response");
        assert_eq!(response.body(), b"ok");
        let requests = requests.lock().expect("lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), Method::Post);
        assert_eq!(requests[0].body_bytes(), &[1, 2, 3]);
    }

    #[test]
    fn production_tls_uses_the_windows_store_without_a_disable_switch() {
        let tls = production_tls();
        assert!(matches!(tls.root_certs(), RootCerts::PlatformVerifier));
        assert!(!tls.disable_verification());
        assert_eq!(tls.provider(), TlsProvider::Rustls);
    }

    #[test]
    fn test_verifier_is_built_from_the_fixture_ca() {
        let tls = tls_with_roots(fixture_ca_roots());
        assert!(!tls.disable_verification());
        match tls.root_certs() {
            RootCerts::Specific(certificates) => assert_eq!(certificates.len(), 1),
            other => panic!("unexpected roots: {other:?}"),
        }
    }

    #[test]
    fn the_production_verifier_rejects_the_test_leaf() {
        let (port, handle) = tls_server();
        let client = HttpClient::with_duration(Duration::from_secs(5));
        let error = client
            .get(&format!("https://127.0.0.1:{port}/"))
            .expect_err("untrusted certificate");
        handle.join().expect("server");
        assert!(error.is_retryable());
    }

    #[test]
    fn the_fixture_ca_verifier_accepts_the_test_leaf() {
        let (port, handle) = tls_server();
        let client =
            HttpClient::with_tls(Duration::from_secs(5), tls_with_roots(fixture_ca_roots()));
        let response = client
            .get(&format!("https://127.0.0.1:{port}/"))
            .expect("trusted response");
        handle.join().expect("server");
        assert_eq!(response.text().expect("text"), "ok");
    }
}
