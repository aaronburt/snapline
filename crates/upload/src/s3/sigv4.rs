use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use snapline_core::secret::Secret;

pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
pub const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    access_key_id: String,
    secret_access_key: Secret,
    session_token: Option<Secret>,
}

impl Credentials {
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<Secret>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
        }
    }

    pub fn with_session_token(mut self, token: impl Into<Secret>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    pub fn session_token(&self) -> Option<&Secret> {
        self.session_token.as_ref()
    }

    fn secret_access_key(&self) -> &[u8] {
        self.secret_access_key.expose()
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"***")
            .field("session_token", &self.session_token.as_ref().map(|_| "***"))
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SignRequest<'a> {
    pub method: &'a str,
    pub canonical_uri: &'a str,
    pub query: &'a [(&'a str, &'a str)],
    pub headers: &'a [(&'a str, &'a str)],
    pub unsigned_headers: &'a [&'a str],
    pub payload_hash: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub authorization: String,
    pub canonical_request: String,
    pub string_to_sign: String,
    pub signed_headers: String,
    pub signature: String,
}

pub fn sign(
    credentials: &Credentials,
    region: &str,
    service: &str,
    amz_date: &str,
    request: &SignRequest<'_>,
) -> Signature {
    let date = amz_date.get(..8).unwrap_or(amz_date);
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let (canonical_headers, signed_headers) =
        canonical_headers(request.headers, request.unsigned_headers);
    let canonical_query = canonical_query(request.query);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method,
        request.canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        request.payload_hash
    );
    let canonical_request_hash = sha256_hex(canonical_request.as_bytes());
    let string_to_sign =
        format!("{ALGORITHM}\n{amz_date}\n{scope}\n{canonical_request_hash}");
    let key = signing_key(credentials.secret_access_key(), date, region, service);
    let signature = hex(&hmac_sha256(&key, string_to_sign.as_bytes()));
    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id()
    );
    Signature {
        authorization,
        canonical_request,
        string_to_sign,
        signed_headers,
        signature,
    }
}

pub fn canonical_headers(
    headers: &[(&str, &str)],
    unsigned_headers: &[&str],
) -> (String, String) {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.trim();
            if name.is_empty() || unsigned_headers.iter().any(|skip| skip.eq_ignore_ascii_case(name))
            {
                return None;
            }
            Some((name.to_ascii_lowercase(), normalize_header_value(value)))
        })
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    let mut combined: Vec<(String, String)> = Vec::new();
    for (name, value) in pairs {
        if let Some(last) = combined.last_mut() {
            if last.0 == name {
                last.1.push(',');
                last.1.push_str(&value);
                continue;
            }
        }
        combined.push((name, value));
    }
    let mut text = String::new();
    let mut names = Vec::new();
    for (name, value) in &combined {
        names.push(name.as_str());
        text.push_str(name);
        text.push(':');
        text.push_str(value);
        text.push('\n');
    }
    (text, names.join(";"))
}

fn normalize_header_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (encode_component(name), encode_component(value)))
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn canonical_uri(path: &str) -> String {
    if path.is_empty() {
        return "/".to_owned();
    }
    let encoded = encode_path(path);
    if encoded.starts_with('/') {
        encoded
    } else {
        format!("/{encoded}")
    }
}

pub fn encode_path(value: &str) -> String {
    encode(value, true)
}

pub fn encode_component(value: &str) -> String {
    encode(value, false)
}

fn encode(value: &str, keep_slash: bool) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if is_unreserved(byte) || (keep_slash && byte == b'/') {
            encoded.push(char::from(byte));
            continue;
        }
        encoded.push('%');
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

pub fn signing_key(secret: &[u8], date: &str, region: &str, service: &str) -> [u8; 32] {
    let mut prefixed = Vec::with_capacity(secret.len() + 4);
    prefixed.extend_from_slice(b"AWS4");
    prefixed.extend_from_slice(secret);
    let date_key = hmac_sha256(&prefixed, date.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, service.as_bytes());
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(key) else {
        return [0_u8; 32];
    };
    mac.update(data);
    let mut output = [0_u8; 32];
    output.copy_from_slice(&mac.finalize().into_bytes());
    output
}

pub fn format_amz_date(unix_seconds: u64) -> String {
    let days = unix_seconds / 86_400;
    let seconds = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(i64::try_from(days).unwrap_or(0));
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        )
    }

    fn s3_credentials() -> Credentials {
        Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        )
    }

    fn sign_suite(request: &SignRequest<'_>) -> Signature {
        sign(&credentials(), "us-east-1", "service", "20150830T123600Z", request)
    }

    #[test]
    fn empty_payload_hash_is_the_sha256_of_nothing() {
        assert_eq!(sha256_hex(b""), EMPTY_PAYLOAD_SHA256);
    }

    #[test]
    fn get_vanilla_matches_the_published_vector() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let signature = sign_suite(&SignRequest {
            method: "GET",
            canonical_uri: "/",
            query: &[],
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert_eq!(signature.signed_headers, "host;x-amz-date");
        assert_eq!(
            signature.canonical_request,
            "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn query_keys_sort_case_sensitively() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let query = [("Param2", "value2"), ("Param1", "value1")];
        let signature = sign_suite(&SignRequest {
            method: "GET",
            canonical_uri: "/",
            query: &query,
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    #[test]
    fn query_values_sort_within_a_repeated_key() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let query = [("Param1", "value2"), ("Param1", "Value1")];
        let signature = sign_suite(&SignRequest {
            method: "GET",
            canonical_uri: "/",
            query: &query,
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "eedbc4e291e521cf13422ffca22be7d2eb8146eecf653089df300a15b2382bd1"
        );
    }

    #[test]
    fn unreserved_path_characters_are_not_encoded() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let signature = sign_suite(&SignRequest {
            method: "GET",
            canonical_uri: "/-._~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
            query: &[],
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "07ef7494c76fa4850883e2b006601f940f8a34d404d0cfa977f52a65bbf5f24f"
        );
    }

    #[test]
    fn header_values_are_trimmed_and_collapsed() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("My-Header1", "value1"),
            ("My-Header2", "\"a   b   c\""),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let signature = sign_suite(&SignRequest {
            method: "GET",
            canonical_uri: "/",
            query: &[],
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "acc3ed3afb60bb290fc8d2dd0098b9911fcaa05412b367055dee359757a9c736"
        );
        assert!(signature
            .canonical_request
            .contains("my-header2:\"a b c\"\n"));
    }

    #[test]
    fn duplicate_headers_join_in_the_order_given() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("My-Header1", "value2"),
            ("My-Header1", "value2"),
            ("My-Header1", "value1"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let signature = sign_suite(&SignRequest {
            method: "GET",
            canonical_uri: "/",
            query: &[],
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "c9d5ea9f3f72853aea855b47ea873832890dbdd183b4468f858259531a5138ea"
        );
        assert!(signature
            .canonical_request
            .contains("my-header1:value2,value2,value1\n"));
    }

    #[test]
    fn unsigned_headers_are_excluded_from_the_signature() {
        let headers = [
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
            ("Content-Length", "13"),
        ];
        let signature = sign_suite(&SignRequest {
            method: "POST",
            canonical_uri: "/",
            query: &[],
            headers: &headers,
            unsigned_headers: &["content-length"],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "ff11897932ad3f4e8b18135d722051e5ac45fc38421b1da7b9d196a0fe09473a"
        );
        assert_eq!(signature.signed_headers, "content-type;host;x-amz-date");
        assert!(!signature.canonical_request.contains("content-length"));
    }

    #[test]
    fn a_security_token_is_signed() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
            (
                "X-Amz-Security-Token",
                "AQoDYXdzEPT//////////wEXAMPLEtc764bNrC9SAPBSM22wDOk4x4HIZ8j4FZTwdQWLWsKWHGBuFqwAeMicRXmxfpSPfIeoIYRqTflfKD8YUuwthAx7mSEI/qkPpKPi/kMcGdQrmGdeehM4IC1NtBmUpp2wUE8phUZampKsburEDy0KPkyQDYwT7WZ0wq5VSXDvp75YU9HFvlRd8Tx6q6fE8YQcHNVXAkiY9q6d+xo0rKwT38xVqr7ZD0u0iPPkUL64lIZbqBAz+scqKmlzm8FDrypNC9Yjc8fPOLn9FX9KSYvKTr4rvx3iSIlTJabIQwj2ICCR/oLxBA==",
            ),
        ];
        let signature = sign_suite(&SignRequest {
            method: "POST",
            canonical_uri: "/",
            query: &[],
            headers: &headers,
            unsigned_headers: &[],
            payload_hash: EMPTY_PAYLOAD_SHA256,
        });
        assert_eq!(
            signature.signature,
            "85d96828115b5dc0cfc3bd16ad9e210dd772bbebba041836c64533a82be05ead"
        );
        assert_eq!(
            signature.signed_headers,
            "host;x-amz-date;x-amz-security-token"
        );
    }

    #[test]
    fn s3_get_object_matches_the_published_vector() {
        let headers = [
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("Range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_PAYLOAD_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let signature = sign(
            &s3_credentials(),
            "us-east-1",
            "s3",
            "20130524T000000Z",
            &SignRequest {
                method: "GET",
                canonical_uri: "/test.txt",
                query: &[],
                headers: &headers,
                unsigned_headers: &[],
                payload_hash: EMPTY_PAYLOAD_SHA256,
            },
        );
        assert_eq!(
            signature.signature,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert_eq!(
            signature.signed_headers,
            "host;range;x-amz-content-sha256;x-amz-date"
        );
    }

    #[test]
    fn s3_put_object_matches_the_published_vector() {
        let payload = "Welcome to Amazon S3.";
        let payload_hash = sha256_hex(payload.as_bytes());
        assert_eq!(
            payload_hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let headers = [
            ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", payload_hash.as_str()),
            ("x-amz-date", "20130524T000000Z"),
            ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
        ];
        let signature = sign(
            &s3_credentials(),
            "us-east-1",
            "s3",
            "20130524T000000Z",
            &SignRequest {
                method: "PUT",
                canonical_uri: "/test%24file.text",
                query: &[],
                headers: &headers,
                unsigned_headers: &[],
                payload_hash: &payload_hash,
            },
        );
        assert_eq!(
            signature.signature,
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
        assert!(signature
            .canonical_request
            .contains("/test%24file.text\n"));
    }

    #[test]
    fn s3_bucket_list_query_matches_the_published_vector() {
        let headers = [
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_PAYLOAD_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let query = [("max-keys", "2"), ("prefix", "J")];
        let signature = sign(
            &s3_credentials(),
            "us-east-1",
            "s3",
            "20130524T000000Z",
            &SignRequest {
                method: "GET",
                canonical_uri: "/",
                query: &query,
                headers: &headers,
                unsigned_headers: &[],
                payload_hash: EMPTY_PAYLOAD_SHA256,
            },
        );
        assert_eq!(
            signature.signature,
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    #[test]
    fn s3_empty_query_value_matches_the_published_vector() {
        let headers = [
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_PAYLOAD_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let query = [("lifecycle", "")];
        let signature = sign(
            &s3_credentials(),
            "us-east-1",
            "s3",
            "20130524T000000Z",
            &SignRequest {
                method: "GET",
                canonical_uri: "/",
                query: &query,
                headers: &headers,
                unsigned_headers: &[],
                payload_hash: EMPTY_PAYLOAD_SHA256,
            },
        );
        assert_eq!(
            signature.signature,
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
    }

    #[test]
    fn canonical_uri_encodes_reserved_bytes_once() {
        assert_eq!(canonical_uri("test$file.text"), "/test%24file.text");
        assert_eq!(
            canonical_uri("/photos/photo 1.jpg"),
            "/photos/photo%201.jpg"
        );
        assert_eq!(canonical_uri(""), "/");
        assert_eq!(canonical_uri("/"), "/");
        assert_eq!(
            canonical_uri("/a/b?c"),
            "/a/b%3Fc"
        );
    }

    #[test]
    fn canonical_query_encodes_spaces_as_percent_twenty() {
        let query = [("b", "2"), ("a", "1"), ("c", "hello world")];
        assert_eq!(canonical_query(&query), "a=1&b=2&c=hello%20world");
        assert_eq!(
            canonical_query(&[("slash", "/"), ("tilde", "~")]),
            "slash=%2F&tilde=~"
        );
    }

    #[test]
    fn amz_dates_are_formatted_in_utc() {
        assert_eq!(format_amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(format_amz_date(0), "19700101T000000Z");
        assert_eq!(format_amz_date(1_700_000_000), "20231114T221320Z");
    }

    #[test]
    fn credentials_debug_redacts_the_secret_and_token() {
        let credentials = s3_credentials().with_session_token("token-value");
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"));
        assert!(!debug.contains("token-value"));
        assert!(debug.contains("***"));
    }

    #[test]
    fn unsigned_header_matching_is_case_insensitive() {
        let headers = [("Host", "example.amazonaws.com"), ("Content-Length", "0")];
        let (canonical, signed) = canonical_headers(&headers, &["CONTENT-LENGTH"]);
        assert_eq!(canonical, "host:example.amazonaws.com\n");
        assert_eq!(signed, "host");
    }
}
