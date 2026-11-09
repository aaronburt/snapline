use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UploadError {
    #[error("{0}")]
    Retryable(String),
    #[error("The file is too large for this destination.")]
    TooLarge,
    #[error("The destination rejected the credentials.")]
    Auth,
    #[error("{0}")]
    Permanent(String),
}

impl UploadError {
    pub fn from_status(status: u16) -> Self {
        Self::from_status_with_retry_after(status, None)
    }

    pub fn from_status_with_retry_after(status: u16, retry_after: Option<Duration>) -> Self {
        match status {
            408 | 429 | 500..=599 => Self::retryable_status(status, retry_after),
            301 | 302 | 303 | 307 | 308 => Self::Permanent(format!(
                "The server redirected the request (HTTP {status}). Check the destination address, including http versus https."
            )),
            413 => Self::TooLarge,
            401 | 403 => Self::Auth,
            _ => Self::Permanent(format!("The server returned HTTP {status}.")),
        }
    }

    pub fn from_io(error: &std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::TimedOut => Self::Retryable("The request timed out.".to_owned()),
            _ => Self::Retryable("The connection failed.".to_owned()),
        }
    }

    pub fn from_ureq(error: &ureq::Error) -> Self {
        use ureq::Error as UreqError;
        match error {
            UreqError::StatusCode(status) => Self::from_status(*status),
            UreqError::Timeout(_) => Self::Retryable("The request timed out.".to_owned()),
            UreqError::Io(_) | UreqError::ConnectionFailed => {
                Self::Retryable("The connection failed.".to_owned())
            }
            UreqError::HostNotFound => Self::Retryable("The server was not found.".to_owned()),
            UreqError::Tls(_) | UreqError::Rustls(_) => {
                Self::Retryable("The secure connection failed.".to_owned())
            }
            UreqError::Protocol(_) => {
                Self::Retryable("The server response could not be read.".to_owned())
            }
            UreqError::BodyExceedsLimit(_) => Self::TooLarge,
            UreqError::BadUri(_) => Self::Permanent("The request address is not valid.".to_owned()),
            UreqError::InvalidProxyUrl => {
                Self::Permanent("The proxy address is not valid.".to_owned())
            }
            UreqError::Http(_) => Self::Permanent("The request could not be built.".to_owned()),
            _ => Self::Permanent("The request failed.".to_owned()),
        }
    }

    pub fn from_json(_error: &serde_json::Error) -> Self {
        Self::Permanent("The server response could not be read.".to_owned())
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        Self::Retryable(message.into())
    }

    pub fn permanent(message: impl Into<String>) -> Self {
        Self::Permanent(message.into())
    }

    fn retryable_status(status: u16, retry_after: Option<Duration>) -> Self {
        match retry_after {
            Some(delay) => Self::Retryable(format!(
                "The server returned HTTP {status}, retry after {} s.",
                delay.as_secs()
            )),
            None => Self::Retryable(format!("The server returned HTTP {status}.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses_map_to_retryable() {
        for status in [408, 429, 500, 502, 503, 599] {
            assert!(UploadError::from_status(status).is_retryable(), "{status}");
        }
    }

    #[test]
    fn payload_too_large_maps_to_too_large() {
        assert_eq!(UploadError::from_status(413), UploadError::TooLarge);
    }

    #[test]
    fn unauthorized_and_forbidden_map_to_auth() {
        assert_eq!(UploadError::from_status(401), UploadError::Auth);
        assert_eq!(UploadError::from_status(403), UploadError::Auth);
    }

    #[test]
    fn other_client_errors_map_to_permanent() {
        for status in [400, 404, 409, 422, 451] {
            assert!(matches!(
                UploadError::from_status(status),
                UploadError::Permanent(_)
            ));
        }
    }

    #[test]
    fn retry_after_is_reported_without_a_secret() {
        let error = UploadError::from_status_with_retry_after(429, Some(Duration::from_secs(2)));
        assert_eq!(
            error,
            UploadError::Retryable("The server returned HTTP 429, retry after 2 s.".to_owned())
        );
    }

    #[test]
    fn io_timeout_maps_to_retryable() {
        let error = UploadError::from_io(&std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert_eq!(
            error,
            UploadError::Retryable("The request timed out.".to_owned())
        );
    }

    #[test]
    fn json_errors_map_to_permanent() {
        let error =
            serde_json::from_slice::<serde_json::Value>(b"not json").expect_err("invalid json");
        assert!(matches!(
            UploadError::from_json(&error),
            UploadError::Permanent(_)
        ));
    }

    #[test]
    fn messages_never_carry_the_original_error() {
        let error = UploadError::from_json(
            &serde_json::from_slice::<serde_json::Value>(b"{\"token\":\"secret\"")
                .expect_err("invalid json"),
        );
        let message = error.to_string();
        assert!(!message.contains("secret"));
    }
}
