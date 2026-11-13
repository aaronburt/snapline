//! Manual "check for updates": asks GitHub for the latest release and compares its tag with
//! the running version. It never downloads anything; the UI just opens the release page.

use crate::error::UploadError;
use crate::http::{HttpClient, HttpTimeout, HttpTransport, ProxySetting, Request};

pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/aaronburt/snapline/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    NoReleases,
    Available { tag: String, url: String },
}

/// Asks GitHub for the latest release and compares it with `current`.
pub fn check(current: &str) -> Result<UpdateStatus, UploadError> {
    let client = HttpClient::new(HttpTimeout::ThirtySeconds, ProxySetting::System)?;
    let request = Request::get(LATEST_RELEASE_URL).header("Accept", "application/vnd.github+json");
    let response = client.execute(request)?;
    interpret(response.status(), response.body(), current)
}

fn interpret(status: u16, body: &[u8], current: &str) -> Result<UpdateStatus, UploadError> {
    match status {
        200 => {}
        404 => return Ok(UpdateStatus::NoReleases),
        403 | 429 => {
            return Err(UploadError::Permanent(
                "GitHub is limiting requests right now. Try again later.".to_owned(),
            ))
        }
        other => return Err(UploadError::from_status(other)),
    }
    let json: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| UploadError::Permanent("GitHub sent an unexpected reply.".to_owned()))?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let url = json["html_url"].as_str().unwrap_or_default();
    if !url.starts_with("https://") {
        return Err(UploadError::Permanent(
            "GitHub sent an unexpected reply.".to_owned(),
        ));
    }
    Ok(if is_newer(tag, current) {
        UpdateStatus::Available {
            tag: tag.to_owned(),
            url: url.to_owned(),
        }
    } else {
        UpdateStatus::UpToDate
    })
}

/// Numeric dotted parts of a tag or version ("v0.12" and "0.12.0" both work). A pre-release
/// or build suffix is ignored. Returns `None` when it is not a version at all.
fn parse(version: &str) -> Option<Vec<u64>> {
    let core = version.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let parts: Option<Vec<u64>> = core.split('.').map(|part| part.parse().ok()).collect();
    parts.filter(|parts| !parts.is_empty())
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(mut latest), Some(mut current)) = (parse(latest), parse(current)) else {
        return false;
    };
    let len = latest.len().max(current.len());
    latest.resize(len, 0);
    current.resize(len, 0);
    latest > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_tags_are_detected_with_or_without_a_patch_part() {
        assert!(is_newer("v0.12", "0.11.0"));
        assert!(is_newer("v0.11", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("v0.11", "0.11.0"));
        assert!(!is_newer("v0.10", "0.11.0"));
        assert!(!is_newer("nightly", "0.11.0"));
    }

    #[test]
    fn a_reply_with_a_newer_tag_offers_the_release_page() {
        let body = br#"{"tag_name":"v0.12","html_url":"https://github.com/aaronburt/snapline/releases/tag/v0.12"}"#;
        assert_eq!(
            interpret(200, body, "0.11.0").unwrap(),
            UpdateStatus::Available {
                tag: "v0.12".to_owned(),
                url: "https://github.com/aaronburt/snapline/releases/tag/v0.12".to_owned()
            }
        );
        assert_eq!(
            interpret(200, body, "0.12.0").unwrap(),
            UpdateStatus::UpToDate
        );
    }

    #[test]
    fn missing_releases_rate_limits_and_bad_replies_are_reported() {
        assert_eq!(
            interpret(404, b"", "0.11.0").unwrap(),
            UpdateStatus::NoReleases
        );
        assert!(interpret(403, b"", "0.11.0").is_err());
        assert!(interpret(200, b"not json", "0.11.0").is_err());
        assert!(interpret(
            200,
            br#"{"tag_name":"v9","html_url":"javascript:x"}"#,
            "0.1.0"
        )
        .is_err());
        assert!(interpret(500, b"", "0.11.0").is_err());
    }
}
