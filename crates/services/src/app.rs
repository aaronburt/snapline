use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_store::history::{
    Capture, CaptureKind, HistoryError, HistoryFilter as StoreFilter, HistoryRepo, UploadState,
};
use ts_rs::TS;

use crate::dto::app::{AppReq, AppResp, RecentCaptureDto, RecentStatusDto};
use crate::dto::history::CaptureKindDto;
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "AppAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
    Navigate { route: String },
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct AppService {
    services: Services,
}

impl AppService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }

    pub fn handle(&self, req: AppReq) -> Result<AppResp, ServiceError> {
        match req {
            AppReq::Ping => Ok(AppResp::Pong),
            AppReq::Ready => Ok(AppResp::Ready),
            AppReq::Info => Ok(AppResp::Info { has_mica: false }),
            AppReq::PanelResize { .. } => Ok(AppResp::PanelResize),
            AppReq::PanelClose => Ok(AppResp::PanelClose),
            AppReq::PanelAction { .. } => Ok(AppResp::PanelAction),
            AppReq::PanelCopyLink => Ok(AppResp::PanelCopyLink),
            AppReq::OpenMain { .. } => Ok(AppResp::OpenMain),
            AppReq::OpenCapture { .. } => Ok(AppResp::OpenCapture),
            AppReq::Recent => Ok(AppResp::Recent {
                captures: self.recent()?,
            }),
            AppReq::Stats => Ok(AppResp::Stats {
                count: self.count()?,
            }),
        }
    }

    fn recent(&self) -> Result<Vec<RecentCaptureDto>, ServiceError> {
        let paths = self.services.paths()?;
        if !paths.history_file.is_file() {
            return Ok(Vec::new());
        }
        let repo = HistoryRepo::open_read(&paths.history_file).map_err(history_error)?;
        let page = repo.page(StoreFilter::All, None).map_err(history_error)?;
        Ok(page.items.iter().take(3).map(recent_dto).collect())
    }

    fn count(&self) -> Result<i64, ServiceError> {
        let paths = self.services.paths()?;
        if !paths.history_file.is_file() {
            return Ok(0);
        }
        let repo = HistoryRepo::open_read(&paths.history_file).map_err(history_error)?;
        Ok(repo.stats().map_err(history_error)?.count)
    }
}

fn recent_dto(capture: &Capture) -> RecentCaptureDto {
    let upload = capture.upload.as_ref();
    let status = match upload.map(|upload| upload.state) {
        Some(UploadState::Done) => RecentStatusDto::Uploaded,
        Some(UploadState::Queued | UploadState::Uploading) => RecentStatusDto::Uploading,
        Some(UploadState::Failed) => RecentStatusDto::Failed,
        Some(UploadState::Cancelled) | None => RecentStatusDto::Local,
    };
    RecentCaptureDto {
        id: capture.id,
        name: capture.name().to_owned(),
        kind: match capture.kind {
            CaptureKind::Image => CaptureKindDto::Image,
            CaptureKind::Video => CaptureKindDto::Video,
            CaptureKind::Gif => CaptureKindDto::Gif,
            CaptureKind::Text => CaptureKindDto::Text,
            CaptureKind::File => CaptureKindDto::File,
        },
        status,
        created_at: capture.created_at,
        url: upload.and_then(|upload| upload.url.clone()),
        duration_ms: capture.duration_ms,
    }
}

fn history_error(error: HistoryError) -> ServiceError {
    match error {
        HistoryError::LocalTimeUnavailable => {
            ServiceError::Internal("Local time is not available on this PC.".to_owned())
        }
        other => {
            tracing::warn!(error = %other, "history could not be read");
            ServiceError::Io("The History could not be read.".to_owned())
        }
    }
}

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}

#[cfg(test)]
mod tests {
    use snapline_store::history::{
        CaptureId, NewCapture, UploadState as StoreUploadState, UploadUpdate,
    };
    use snapline_store::io::Paths;
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        _directory: TempDir,
        services: Services,
        paths: Paths,
    }

    fn fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("temp directory");
        let root = directory.path();
        let paths = Paths::resolve(
            &root.join("app"),
            &root.join("roaming"),
            &root.join("local"),
            &root.join("profile"),
            false,
        );
        std::fs::create_dir_all(&paths.captures_dir).expect("captures folder");
        let services = Services::with_paths(paths.clone()).expect("services");
        Fixture {
            _directory: directory,
            services,
            paths,
        }
    }

    impl Fixture {
        fn service(&self) -> AppService {
            self.services.app()
        }

        fn repo(&self) -> HistoryRepo {
            HistoryRepo::open_write(&self.paths.history_file).expect("history")
        }

        fn capture(
            &self,
            name: &str,
            kind: CaptureKind,
            created_at: i64,
            duration_ms: Option<i64>,
        ) -> CaptureId {
            let path = self.paths.captures_dir.join("2026-11").join(name);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("capture folder");
            std::fs::write(&path, b"capture bytes").expect("capture file");
            self.repo()
                .record(NewCapture {
                    kind,
                    path: path.to_string_lossy().into_owned(),
                    bytes: 13,
                    width: Some(1280),
                    height: Some(720),
                    duration_ms,
                    source_app: Some("chrome.exe".to_owned()),
                    created_at,
                    ocr_text: None,
                })
                .expect("record")
        }

        fn upload(&self, id: CaptureId, state: StoreUploadState, url: Option<&str>) {
            self.repo()
                .set_upload(UploadUpdate {
                    sent: 0,
                    total: 0,
                    capture_id: id,
                    dest_id: "imgur".to_owned(),
                    state,
                    url: url.map(str::to_owned),
                    deletion_url: None,
                    error: None,
                    attempts: 1,
                    updated_at: 1_760_000_100_000,
                })
                .expect("upload");
        }
    }

    const BASE: i64 = 1_760_000_000_000;

    fn captures(response: AppResp) -> Vec<RecentCaptureDto> {
        match response {
            AppResp::Recent { captures } => captures,
            other => panic!("the wrong response was returned: {other:?}"),
        }
    }

    #[test]
    fn recent_returns_the_three_newest_captures() {
        let fixture = fixture();
        fixture.capture("old.png", CaptureKind::Image, BASE, None);
        fixture.capture("second.png", CaptureKind::Image, BASE + 2_000, None);
        fixture.capture("clip.gif", CaptureKind::Gif, BASE + 1_000, Some(8_000));
        fixture.capture("first.png", CaptureKind::Image, BASE + 4_000, None);
        fixture.capture("third.mp4", CaptureKind::Video, BASE + 3_000, Some(42_000));
        let response = fixture.service().handle(AppReq::Recent).expect("recent");
        let recent = captures(response);
        assert_eq!(
            recent
                .iter()
                .map(|capture| capture.name.as_str())
                .collect::<Vec<_>>(),
            vec!["first.png", "third.mp4", "second.png"]
        );
        let oldest = &recent[2];
        assert_eq!(oldest.status, RecentStatusDto::Local);
        assert_eq!(oldest.duration_ms, None);
        let previous = &recent[1];
        assert_eq!(previous.kind, CaptureKindDto::Video);
        assert_eq!(previous.duration_ms, Some(42_000));
        assert_eq!(previous.url, None);
    }

    #[test]
    fn recent_carries_the_upload_status_and_url() {
        let fixture = fixture();
        let id = fixture.capture("linked.png", CaptureKind::Image, BASE, None);
        fixture.upload(
            id,
            StoreUploadState::Done,
            Some("https://example.com/linked.png"),
        );
        let recent = captures(fixture.service().handle(AppReq::Recent).expect("recent"));
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].status, RecentStatusDto::Uploaded);
        assert_eq!(
            recent[0].url.as_deref(),
            Some("https://example.com/linked.png")
        );
    }

    #[test]
    fn recent_is_empty_when_there_is_no_history() {
        let fixture = fixture();
        let recent = captures(fixture.service().handle(AppReq::Recent).expect("recent"));
        assert!(recent.is_empty());
        assert_eq!(
            fixture.service().handle(AppReq::Stats).expect("stats"),
            AppResp::Stats { count: 0 }
        );
    }

    #[test]
    fn stats_count_every_capture() {
        let fixture = fixture();
        fixture.capture("a.png", CaptureKind::Image, BASE, None);
        fixture.capture("b.mp4", CaptureKind::Video, BASE + 1, Some(1_000));
        fixture.capture("c.gif", CaptureKind::Gif, BASE + 2, Some(2_000));
        assert_eq!(
            fixture.service().handle(AppReq::Stats).expect("stats"),
            AppResp::Stats { count: 3 }
        );
    }

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(services.app().handle(AppReq::Ping), Ok(AppResp::Pong));
    }

    #[test]
    fn panel_requests_return_typed_responses() {
        let services = Services::new();
        assert_eq!(
            services.app().handle(AppReq::PanelResize { height: 321.0 }),
            Ok(AppResp::PanelResize)
        );
        assert_eq!(
            services.app().handle(AppReq::PanelClose),
            Ok(AppResp::PanelClose)
        );
        assert_eq!(
            services.app().handle(AppReq::PanelAction {
                action: "capture_region".to_string()
            }),
            Ok(AppResp::PanelAction)
        );
        assert_eq!(
            services.app().handle(AppReq::PanelCopyLink),
            Ok(AppResp::PanelCopyLink)
        );
        assert_eq!(
            services.app().handle(AppReq::OpenMain {
                route: "/history".to_string()
            }),
            Ok(AppResp::OpenMain)
        );
        assert_eq!(
            services.app().handle(AppReq::OpenCapture { id: 1 }),
            Ok(AppResp::OpenCapture)
        );
    }
}
