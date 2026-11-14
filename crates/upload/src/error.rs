use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UploadError {
    #[error("{0}")]
    Retryable(String),
    #[error("{0}")]
    Uncertain(String),
    #[error("The file is too large for this destination.")]
    TooLarge,
    #[error("The upload was cancelled.")]
    Cancelled,
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
            408 | 429 | 500..=599 => Self::transient_status(status, retry_after, None),
            301 | 302 | 303 | 307 | 308 => Self::Permanent(format!(
                "The server redirected the request (HTTP {status}). Check the destination address, including http versus https."
            )),
            413 => Self::TooLarge,
            401 | 403 => Self::Auth,
            _ => Self::Permanent(format!("The server returned HTTP {status}.")),
        }
    }

    pub fn transient_status(
        status: u16,
        retry_after: Option<Duration>,
        message: Option<String>,
    ) -> Self {
        let declared_safe = retry_after.is_some() && matches!(status, 429 | 503);
        let text = match (message, retry_after) {
            (Some(message), _) => message,
            (None, Some(delay)) => format!(
                "The server returned HTTP {status}, retry after {} s.",
                delay.as_secs()
            ),
            (None, None) => format!("The server returned HTTP {status}."),
        };
        if declared_safe {
            Self::Retryable(text)
        } else {
            Self::Uncertain(text)
        }
    }

    pub fn from_io(error: &std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::TimedOut => Self::Uncertain("The request timed out.".to_owned()),
            _ => Self::Uncertain("The connection failed.".to_owned()),
        }
    }

    fn from_transport_io(error: &std::io::Error) -> Self {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::ConnectionRefused
            | ErrorKind::NotConnected
            | ErrorKind::AddrNotAvailable
            | ErrorKind::InvalidData => Self::Retryable("The connection failed.".to_owned()),
            _ => Self::Uncertain("The connection failed.".to_owned()),
        }
    }

    pub fn from_ureq(error: &ureq::Error) -> Self {
        use ureq::Error as UreqError;
        use ureq::Timeout;
        match error {
            UreqError::StatusCode(status) => Self::from_status(*status),
            UreqError::Timeout(Timeout::Resolve | Timeout::Connect | Timeout::SendRequest) => {
                Self::Retryable("The request timed out.".to_owned())
            }
            UreqError::Timeout(_) => Self::Uncertain("The request timed out.".to_owned()),
            UreqError::ConnectionFailed => Self::Retryable("The connection failed.".to_owned()),
            UreqError::Io(error) => Self::from_transport_io(error),
            UreqError::HostNotFound => Self::Retryable("The server was not found.".to_owned()),
            UreqError::Tls(_) | UreqError::Rustls(_) => {
                Self::Retryable("The secure connection failed.".to_owned())
            }
            UreqError::Protocol(_) => {
                Self::Uncertain("The server response could not be read.".to_owned())
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

    pub fn is_uncertain(&self) -> bool {
        matches!(self, Self::Uncertain(_))
    }

    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Retryable(_) | Self::Uncertain(_))
    }

    pub fn into_uncertain(self) -> Self {
        match self {
            Self::Retryable(message) => Self::Uncertain(message),
            other => other,
        }
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        Self::Retryable(message.into())
    }

    pub fn uncertain(message: impl Into<String>) -> Self {
        Self::Uncertain(message.into())
    }

    pub fn permanent(message: impl Into<String>) -> Self {
        Self::Permanent(message.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transient_status_without_a_retry_after_is_uncertain() {
        for status in [408, 429, 500, 502, 503, 504, 599] {
            let error = UploadError::from_status(status);
            assert!(error.is_uncertain(), "{status}");
            assert!(error.is_transient(), "{status}");
            assert!(!error.is_retryable(), "{status}");
        }
    }

    #[test]
    fn only_a_declared_retry_after_on_429_or_503_is_safe() {
        let delay = Some(Duration::from_secs(5));
        for status in [429, 503] {
            assert!(
                UploadError::from_status_with_retry_after(status, delay).is_retryable(),
                "{status}"
            );
        }
        for status in [408, 500, 502, 504] {
            assert!(
                UploadError::from_status_with_retry_after(status, delay).is_uncertain(),
                "{status}"
            );
        }
    }

    #[test]
    fn a_message_from_the_server_keeps_the_same_safety_class() {
        let delay = Some(Duration::from_secs(1));
        assert_eq!(
            UploadError::transient_status(503, delay, Some("Busy.".to_owned())),
            UploadError::Retryable("Busy.".to_owned())
        );
        assert_eq!(
            UploadError::transient_status(503, None, Some("Busy.".to_owned())),
            UploadError::Uncertain("Busy.".to_owned())
        );
    }

    #[test]
    fn timeouts_before_the_body_is_sent_are_safe_and_later_ones_are_not() {
        use ureq::Timeout;
        for timeout in [Timeout::Resolve, Timeout::Connect, Timeout::SendRequest] {
            assert!(
                UploadError::from_ureq(&ureq::Error::Timeout(timeout)).is_retryable(),
                "{timeout:?}"
            );
        }
        for timeout in [
            Timeout::SendBody,
            Timeout::RecvResponse,
            Timeout::RecvBody,
            Timeout::Global,
            Timeout::PerCall,
            Timeout::PerRead,
            Timeout::PerWrite,
        ] {
            assert!(
                UploadError::from_ureq(&ureq::Error::Timeout(timeout)).is_uncertain(),
                "{timeout:?}"
            );
        }
    }

    #[test]
    fn connection_setup_failures_are_safe_and_dropped_exchanges_are_not() {
        assert!(UploadError::from_ureq(&ureq::Error::ConnectionFailed).is_retryable());
        assert!(UploadError::from_ureq(&ureq::Error::HostNotFound).is_retryable());
        for kind in [
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::TimedOut,
        ] {
            let dropped = ureq::Error::Io(std::io::Error::from(kind));
            assert!(UploadError::from_ureq(&dropped).is_uncertain(), "{kind:?}");
        }
        let refused = ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
        assert!(UploadError::from_ureq(&refused).is_retryable());
    }

    #[test]
    fn into_uncertain_only_downgrades_a_safe_retry() {
        assert_eq!(
            UploadError::retryable("x").into_uncertain(),
            UploadError::Uncertain("x".to_owned())
        );
        assert_eq!(UploadError::Auth.into_uncertain(), UploadError::Auth);
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
    fn io_timeout_maps_to_uncertain() {
        let error = UploadError::from_io(&std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert_eq!(
            error,
            UploadError::Uncertain("The request timed out.".to_owned())
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
