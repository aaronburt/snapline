use snapline_services::dto::upload::{
    DestinationActionDto, DestinationKindDto, DestinationViewDto,
};

use crate::labels::Pairs;

pub const RETRY: Pairs = &[
    ("never", "Don't retry"),
    ("once", "Once"),
    ("three_times", "3 times"),
    ("five_times", "5 times"),
];
pub const TIMEOUT: Pairs = &[("30s", "30 s"), ("60s", "60 s"), ("120s", "120 s")];
pub const KEEP_HISTORY: Pairs = &[
    ("forever", "Forever"),
    ("one_year", "1 year"),
    ("days90", "90 days"),
    ("days30", "30 days"),
];
pub const PROXY: Pairs = &[
    ("system", "Use system proxy"),
    ("none", "No proxy"),
    ("custom", "Custom proxy…"),
];

pub const SAVE_LOCALLY: (&str, &str) = ("save_locally_only", "Save locally only");
pub const NONE: (&str, &str) = ("none", "None");

pub struct DestinationItem {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub status: String,
    pub configured: bool,
    pub kind: String,
    pub action: String,
}

impl From<&DestinationViewDto> for DestinationItem {
    fn from(view: &DestinationViewDto) -> Self {
        let kind = match view.kind {
            DestinationKindDto::S3 => "s3",
            DestinationKindDto::Gist => "gist",
            DestinationKindDto::Sftp => "sftp",
            DestinationKindDto::Custom => "custom",
        };
        let action = match view.action {
            DestinationActionDto::Configure => "Configure",
            DestinationActionDto::Edit => "Edit",
            DestinationActionDto::SetUp => "Set up",
        };
        Self {
            id: view.id.clone(),
            name: view.name.clone(),
            detail: view.detail.clone(),
            status: if view.configured {
                "Configured".into()
            } else {
                "Not set up".into()
            },
            configured: view.configured,
            kind: kind.to_string(),
            action: action.to_string(),
        }
    }
}

pub fn accepts(kind: &str, target: &str) -> bool {
    match target {
        "images" => kind != "gist",
        "text" => kind == "gist" || kind == "custom",
        _ => kind == "s3" || kind == "sftp" || kind == "custom",
    }
}

pub fn options(target: &str, destinations: &[DestinationItem]) -> Vec<(String, String)> {
    if target == "shortener" {
        return vec![(NONE.0.into(), NONE.1.into())];
    }
    let mut out = vec![(SAVE_LOCALLY.0.into(), SAVE_LOCALLY.1.into())];
    out.extend(
        destinations
            .iter()
            .filter(|item| item.configured && accepts(&item.kind, target))
            .map(|item| (item.id.clone(), item.name.clone())),
    );
    out
}

pub fn label_for(stored: &str, destinations: &[DestinationItem]) -> String {
    if stored == SAVE_LOCALLY.0 {
        return SAVE_LOCALLY.1.into();
    }
    if stored == NONE.0 {
        return NONE.1.into();
    }
    destinations
        .iter()
        .find(|item| item.id == stored || item.name == stored)
        .map(|item| item.name.clone())
        .unwrap_or_else(|| stored.to_string())
}

pub fn value_for(label: &str, options: &[(String, String)]) -> String {
    options
        .iter()
        .find(|(_, shown)| shown == label)
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dest(id: &str, name: &str, kind: &str, configured: bool) -> DestinationItem {
        DestinationItem {
            id: id.to_owned(),
            name: name.to_owned(),
            detail: String::new(),
            status: String::new(),
            configured,
            kind: kind.to_owned(),
            action: String::new(),
        }
    }

    #[test]
    fn options_filter_by_target_and_configured_state() {
        let dests = vec![
            dest("s3", "Amazon S3", "s3", true),
            dest("gist", "GitHub Gist", "gist", true),
            dest("sftp-unconf", "My SFTP", "sftp", false),
            dest("custom", "Custom Host", "custom", true),
        ];

        assert_eq!(
            options("images", &dests),
            vec![
                ("save_locally_only".into(), "Save locally only".into()),
                ("s3".into(), "Amazon S3".into()),
                ("custom".into(), "Custom Host".into()),
            ]
        );
        assert_eq!(
            options("text", &dests),
            vec![
                ("save_locally_only".into(), "Save locally only".into()),
                ("gist".into(), "GitHub Gist".into()),
                ("custom".into(), "Custom Host".into()),
            ]
        );
        assert_eq!(
            options("videos", &dests),
            vec![
                ("save_locally_only".into(), "Save locally only".into()),
                ("s3".into(), "Amazon S3".into()),
                ("custom".into(), "Custom Host".into()),
            ]
        );
        assert_eq!(
            options("shortener", &dests),
            vec![("none".into(), "None".into())]
        );
    }

    #[test]
    fn label_for_and_value_for_roundtrip_and_fall_back() {
        let dests = vec![dest("s3", "Amazon S3", "s3", true)];
        let opts = options("images", &dests);
        assert_eq!(label_for("save_locally_only", &dests), "Save locally only");
        assert_eq!(label_for("none", &dests), "None");
        assert_eq!(label_for("s3", &dests), "Amazon S3");
        assert_eq!(label_for("missing", &dests), "missing");

        assert_eq!(value_for("Amazon S3", &opts), "s3");
        assert_eq!(value_for("Save locally only", &opts), "save_locally_only");
        assert_eq!(value_for("unknown", &opts), "unknown");
    }
}
