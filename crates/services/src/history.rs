use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::image::ImageFormat;
use snapline_store::history::{
    Capture, CaptureKind, Cursor as StoreCursor, HistoryError, HistoryFilter as StoreFilter,
    HistoryPage as StorePage, HistoryRepo, UploadState,
};
use snapline_store::io::Paths;
use ts_rs::TS;

use crate::dto::history::{
    CaptureDto, CaptureKindDto, Cursor, HistoryFilter, HistoryPage, HistoryReq, HistoryResp,
    HistoryStats, UploadDto, UploadStateDto,
};
use crate::{ServiceError, Services};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "HistoryAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
    Added {
        #[ts(as = "i32")]
        id: i64,
    },
    Removed {
        #[ts(as = "i32")]
        id: i64,
    },
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub trait HistoryHost: Send + Sync {
    fn confirm_delete(&self, message: &str) -> Result<bool, ServiceError>;
    fn set_clipboard_image(&self, bytes: &[u8]) -> Result<(), ServiceError>;
    fn show_in_folder(&self, path: &Path) -> Result<(), ServiceError>;
    fn open_file(&self, path: &Path) -> Result<(), ServiceError>;
    fn open_url(&self, url: &str) -> Result<(), ServiceError>;
    fn recycle_file(&self, path: &Path) -> Result<(), ServiceError>;
}

pub struct HistoryService {
    services: Services,
    host: Option<Arc<dyn HistoryHost>>,
}

impl HistoryService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
            host: services.history_host(),
        }
    }

    pub fn with_host(services: &Services, host: Arc<dyn HistoryHost>) -> Self {
        Self {
            services: services.clone(),
            host: Some(host),
        }
    }

    pub fn handle(&self, req: HistoryReq) -> Result<HistoryResp, ServiceError> {
        match req {
            HistoryReq::Ping => Ok(HistoryResp::Pong),
            HistoryReq::Page { filter, cursor } => {
                let paths = self.paths()?;
                let page = match self.reader(&paths)? {
                    Some(repo) => repo
                        .page(store_filter(filter), store_cursor(cursor))
                        .map_err(read_error)?,
                    None => empty_page(),
                };
                Ok(HistoryResp::Page {
                    page: page_dto(page, &paths),
                })
            }
            HistoryReq::Search {
                text,
                filter,
                cursor,
            } => {
                let paths = self.paths()?;
                let page = match self.reader(&paths)? {
                    Some(repo) => repo
                        .search(&text, store_filter(filter), store_cursor(cursor))
                        .map_err(read_error)?,
                    None => empty_page(),
                };
                Ok(HistoryResp::Search {
                    page: page_dto(page, &paths),
                })
            }
            HistoryReq::Get { id } => {
                let paths = self.paths()?;
                let capture = self.capture(&paths, id)?;
                Ok(HistoryResp::Get {
                    capture: capture_dto(&capture, &paths),
                })
            }
            HistoryReq::Stats => {
                let paths = self.paths()?;
                let stats = match self.reader(&paths)? {
                    Some(repo) => repo.stats().map_err(read_error)?,
                    None => snapline_store::history::HistoryStats {
                        count: 0,
                        bytes: 0,
                        uploaded_this_week: 0,
                    },
                };
                Ok(HistoryResp::Stats {
                    stats: HistoryStats {
                        count: stats.count,
                        bytes: stats.bytes,
                        uploaded_this_week: stats.uploaded_this_week,
                    },
                })
            }
            HistoryReq::ConfirmDelete { id } => {
                let paths = self.paths()?;
                let capture = self.capture(&paths, id)?;
                let message = if capture.owned {
                    format!(
                        "Delete {}? The file goes to the Recycle Bin.",
                        file_name(&capture)
                    )
                } else {
                    format!(
                        "Delete {} from History? The original file stays where it is.",
                        file_name(&capture)
                    )
                };
                let confirmed = self.host()?.confirm_delete(&message)?;
                Ok(HistoryResp::ConfirmDelete { confirmed })
            }
            HistoryReq::Delete { id } => {
                let paths = self.paths()?;
                self.delete(&paths, id)?;
                Ok(HistoryResp::Delete { id })
            }
            HistoryReq::CopyImage { id } => {
                let paths = self.paths()?;
                let capture = self.capture(&paths, id)?;
                if capture.kind != CaptureKind::Image {
                    return Err(ServiceError::Invalid(
                        "Only images can be copied to the clipboard.".to_owned(),
                    ));
                }
                let bytes = fs::read(&capture.path).map_err(|error| {
                    tracing::warn!(%error, "history capture could not be read for the clipboard");
                    ServiceError::Io("The image could not be read.".to_owned())
                })?;
                self.host()?.set_clipboard_image(&bytes)?;
                Ok(HistoryResp::CopyImage)
            }
            HistoryReq::ShowInFolder { id } => {
                let paths = self.paths()?;
                let capture = self.capture(&paths, id)?;
                self.host()?.show_in_folder(Path::new(&capture.path))?;
                Ok(HistoryResp::ShowInFolder)
            }
            HistoryReq::OpenFile { id } => {
                let paths = self.paths()?;
                let capture = self.capture(&paths, id)?;
                let path = Path::new(&capture.path);
                if capture.owned {
                    self.host()?.open_file(path)?;
                } else {
                    self.host()?.show_in_folder(path)?;
                }
                Ok(HistoryResp::OpenFile)
            }
            HistoryReq::OpenUrl { id } => {
                let paths = self.paths()?;
                let capture = self.capture(&paths, id)?;
                let url = capture
                    .upload
                    .as_ref()
                    .and_then(|upload| upload.url.as_deref())
                    .ok_or_else(|| ServiceError::Invalid("This capture has no link.".to_owned()))?;
                if !is_web_url(url) {
                    return Err(ServiceError::Invalid(
                        "Only http and https links can be opened.".to_owned(),
                    ));
                }
                self.host()?.open_url(url)?;
                Ok(HistoryResp::OpenUrl)
            }
        }
    }

    fn paths(&self) -> Result<Paths, ServiceError> {
        let state = self.services.lock_settings()?;
        state.paths.clone().ok_or_else(|| {
            ServiceError::Io(
                "History is unavailable because Snapline has no data folder.".to_owned(),
            )
        })
    }

    fn reader(&self, paths: &Paths) -> Result<Option<HistoryRepo>, ServiceError> {
        if !paths.history_file.is_file() {
            return Ok(None);
        }
        HistoryRepo::open_read(&paths.history_file)
            .map(Some)
            .map_err(read_error)
    }

    fn capture(&self, paths: &Paths, id: i64) -> Result<Capture, ServiceError> {
        let repo = self.reader(paths)?.ok_or(ServiceError::NotFound)?;
        repo.get(id)
            .map_err(read_error)?
            .ok_or(ServiceError::NotFound)
    }

    fn delete(&self, paths: &Paths, id: i64) -> Result<(), ServiceError> {
        if !paths.history_file.is_file() {
            return Err(ServiceError::NotFound);
        }
        let mut repo = HistoryRepo::open_write(&paths.history_file).map_err(delete_error)?;
        let capture = repo
            .get(id)
            .map_err(delete_error)?
            .ok_or(ServiceError::NotFound)?;
        let path = PathBuf::from(&capture.path);
        if capture.owned && path.exists() {
            self.host()?.recycle_file(&path)?;
        }
        let thumbnail = crate::thumbs::thumbnail_path(&paths.thumbs_dir, id, ImageFormat::WebP);
        if let Err(error) = fs::remove_file(&thumbnail) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, "history thumbnail could not be removed");
            }
        }
        let fallback = crate::thumbs::thumbnail_path(&paths.thumbs_dir, id, ImageFormat::Jpeg);
        if let Err(error) = fs::remove_file(&fallback) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, "history thumbnail could not be removed");
            }
        }
        repo.delete(id).map_err(delete_error)?;
        Ok(())
    }

    fn host(&self) -> Result<&dyn HistoryHost, ServiceError> {
        self.host
            .as_deref()
            .ok_or_else(|| ServiceError::Io("History actions are unavailable.".to_owned()))
    }
}

pub fn run(_action: &ActionId, _services: &Services) -> Result<(), ServiceError> {
    Err(ServiceError::NotYet)
}

fn store_filter(filter: HistoryFilter) -> StoreFilter {
    match filter {
        HistoryFilter::All => StoreFilter::All,
        HistoryFilter::Images => StoreFilter::Images,
        HistoryFilter::Videos => StoreFilter::Videos,
        HistoryFilter::Gifs => StoreFilter::Gifs,
        HistoryFilter::Files => StoreFilter::Files,
        HistoryFilter::Uploaded => StoreFilter::Uploaded,
        HistoryFilter::LocalOnly => StoreFilter::LocalOnly,
    }
}

fn store_cursor(cursor: Option<Cursor>) -> Option<StoreCursor> {
    cursor.map(|cursor| StoreCursor {
        created_at: cursor.created_at,
        id: cursor.id,
    })
}

fn empty_page() -> StorePage {
    StorePage {
        items: Vec::new(),
        next: None,
    }
}

fn page_dto(page: StorePage, paths: &Paths) -> HistoryPage {
    HistoryPage {
        items: page
            .items
            .iter()
            .map(|capture| capture_dto(capture, paths))
            .collect(),
        next: page.next.map(|cursor| Cursor {
            created_at: cursor.created_at,
            id: cursor.id,
        }),
    }
}

fn capture_dto(capture: &Capture, paths: &Paths) -> CaptureDto {
    CaptureDto {
        id: capture.id,
        kind: match capture.kind {
            CaptureKind::Image => CaptureKindDto::Image,
            CaptureKind::Video => CaptureKindDto::Video,
            CaptureKind::Gif => CaptureKindDto::Gif,
            CaptureKind::Text => CaptureKindDto::Text,
            CaptureKind::File => CaptureKindDto::File,
        },
        owned: capture.owned,
        missing: !Path::new(&capture.path).is_file(),
        name: file_name(capture),
        bytes: capture.bytes,
        width: capture.width,
        height: capture.height,
        duration_ms: capture.duration_ms,
        source_app: capture
            .source_app
            .as_deref()
            .map(snapline_core::naming::app_name),
        created_at: capture.created_at,
        folder: if capture.owned {
            saved_folder(&capture.path, &paths.captures_dir)
        } else {
            String::new()
        },
        upload: capture.upload.as_ref().map(|upload| UploadDto {
            dest_id: upload.dest_id.clone(),
            state: upload_state(upload.state),
            sent: upload.sent,
            total: upload.total,
            url: upload.url.clone(),
            deletion_url: upload.deletion_url.clone(),
            error: upload.error.clone(),
            attempts: upload.attempts,
            updated_at: upload.updated_at,
        }),
    }
}

fn upload_state(state: UploadState) -> UploadStateDto {
    match state {
        UploadState::Queued => UploadStateDto::Queued,
        UploadState::Uploading => UploadStateDto::Uploading,
        UploadState::Done => UploadStateDto::Done,
        UploadState::Failed => UploadStateDto::Failed,
        UploadState::Cancelled => UploadStateDto::Cancelled,
    }
}

fn file_name(capture: &Capture) -> String {
    capture.name().to_owned()
}

fn saved_folder(path: &str, captures_dir: &Path) -> String {
    let folder = Path::new(path).parent().unwrap_or(Path::new(""));
    let profile = captures_dir.parent().and_then(Path::parent);
    let relative = profile
        .and_then(|profile| folder.strip_prefix(profile).ok())
        .unwrap_or(folder);
    let shown = relative.to_string_lossy();
    if shown.is_empty() {
        return String::new();
    }
    shown.replace('/', "\\")
}

fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty())
        && !url
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        && !url
            .chars()
            .any(|character| matches!(character, '"' | '\'' | '`' | '<' | '>'))
}

fn read_error(error: HistoryError) -> ServiceError {
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

fn delete_error(error: HistoryError) -> ServiceError {
    tracing::warn!(error = %error, "history capture could not be deleted");
    ServiceError::Io("The capture could not be deleted.".to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use snapline_store::history::{
        CaptureId, NewCapture, UploadState as StoreUploadState, UploadUpdate,
    };
    use tempfile::TempDir;

    use super::*;

    #[derive(Default)]
    struct FakeHost {
        calls: Mutex<Vec<String>>,
        image: Mutex<Vec<u8>>,
        confirmed: bool,
        fail_recycle: bool,
    }

    impl FakeHost {
        fn record(&self, call: String) {
            self.calls.lock().expect("calls").push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl HistoryHost for FakeHost {
        fn confirm_delete(&self, message: &str) -> Result<bool, ServiceError> {
            self.record(format!("confirm:{message}"));
            Ok(self.confirmed)
        }

        fn set_clipboard_image(&self, bytes: &[u8]) -> Result<(), ServiceError> {
            self.record("copy".to_owned());
            *self.image.lock().expect("image") = bytes.to_vec();
            Ok(())
        }

        fn show_in_folder(&self, path: &Path) -> Result<(), ServiceError> {
            self.record(format!("show:{}", path.display()));
            Ok(())
        }

        fn open_file(&self, path: &Path) -> Result<(), ServiceError> {
            self.record(format!("open:{}", path.display()));
            Ok(())
        }

        fn open_url(&self, url: &str) -> Result<(), ServiceError> {
            self.record(format!("url:{url}"));
            Ok(())
        }

        fn recycle_file(&self, path: &Path) -> Result<(), ServiceError> {
            self.record(format!("recycle:{}", path.display()));
            if self.fail_recycle {
                return Err(ServiceError::Io(
                    "The file could not be recycled.".to_owned(),
                ));
            }
            Ok(())
        }
    }

    struct Fixture {
        _directory: TempDir,
        services: Services,
        paths: Paths,
        host: Arc<FakeHost>,
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
        let host = Arc::new(FakeHost::default());
        Fixture {
            _directory: directory,
            services,
            paths,
            host,
        }
    }

    impl Fixture {
        fn service(&self) -> HistoryService {
            HistoryService::with_host(&self.services, self.host.clone())
        }

        fn repo(&self) -> HistoryRepo {
            HistoryRepo::open_write(&self.paths.history_file).expect("history")
        }

        fn capture(&self, name: &str, kind: CaptureKind, created_at: i64) -> (CaptureId, PathBuf) {
            let path = self.paths.captures_dir.join("2026-11").join(name);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("capture folder");
            std::fs::write(&path, b"capture bytes").expect("capture file");
            let id = self
                .repo()
                .record(NewCapture {
                    kind,
                    path: path.to_string_lossy().into_owned(),
                    bytes: 13,
                    width: Some(1280),
                    height: Some(720),
                    duration_ms: None,
                    source_app: Some("chrome.exe".to_owned()),
                    created_at,
                    ocr_text: None,
                })
                .expect("record");
            (id, path)
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

    #[test]
    fn a_page_returns_camel_case_captures_with_folders() {
        let fixture = fixture();
        let (id, _path) = fixture.capture("one.png", CaptureKind::Image, BASE);
        fixture.upload(
            id,
            StoreUploadState::Done,
            Some("https://example.com/one.png"),
        );
        let response = fixture
            .service()
            .handle(HistoryReq::Page {
                filter: HistoryFilter::All,
                cursor: None,
            })
            .expect("page");
        let HistoryResp::Page { page } = response else {
            panic!("the wrong response was returned");
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, id);
        assert_eq!(page.items[0].name, "one.png");
        assert_eq!(page.items[0].folder, "Pictures\\Snapline\\2026-11");
        assert_eq!(page.next, None);
        let upload = page.items[0].upload.as_ref().expect("upload");
        assert_eq!(upload.state, UploadStateDto::Done);
        assert_eq!(upload.url.as_deref(), Some("https://example.com/one.png"));
    }

    #[test]
    fn a_missing_database_reads_as_empty() {
        let fixture = fixture();
        let service = fixture.service();
        let response = service
            .handle(HistoryReq::Page {
                filter: HistoryFilter::All,
                cursor: None,
            })
            .expect("page");
        let HistoryResp::Page { page } = response else {
            panic!("the wrong response was returned");
        };
        assert!(page.items.is_empty());
        assert_eq!(page.next, None);
        let response = service.handle(HistoryReq::Stats).expect("stats");
        let HistoryResp::Stats { stats } = response else {
            panic!("the wrong response was returned");
        };
        assert_eq!(stats.count, 0);
        assert_eq!(stats.bytes, 0);
        assert_eq!(stats.uploaded_this_week, 0);
    }

    #[test]
    fn get_returns_one_capture_and_missing_ids_are_not_found() {
        let fixture = fixture();
        let (id, _path) = fixture.capture("two.png", CaptureKind::Image, BASE);
        let service = fixture.service();
        let response = service.handle(HistoryReq::Get { id }).expect("get");
        let HistoryResp::Get { capture } = response else {
            panic!("the wrong response was returned");
        };
        assert_eq!(capture.name, "two.png");
        assert_eq!(capture.width, Some(1280));
        assert_eq!(capture.height, Some(720));
        assert_eq!(capture.source_app.as_deref(), Some("chrome"));
        assert_eq!(
            service.handle(HistoryReq::Get { id: id + 1 }),
            Err(ServiceError::NotFound)
        );
    }

    #[test]
    fn stats_count_the_recorded_captures() {
        let fixture = fixture();
        fixture.capture("a.png", CaptureKind::Image, BASE);
        fixture.capture("b.mp4", CaptureKind::Video, BASE + 1);
        let response = fixture.service().handle(HistoryReq::Stats).expect("stats");
        let HistoryResp::Stats { stats } = response else {
            panic!("the wrong response was returned");
        };
        assert_eq!(stats.count, 2);
        assert_eq!(stats.bytes, 26);
    }

    #[test]
    fn search_matches_names_and_filters() {
        let fixture = fixture();
        fixture.capture("Screenshot_alpha.png", CaptureKind::Image, BASE);
        fixture.capture("Recording_alpha.mp4", CaptureKind::Video, BASE + 1);
        let service = fixture.service();
        let response = service
            .handle(HistoryReq::Search {
                text: "screens".to_owned(),
                filter: HistoryFilter::All,
                cursor: None,
            })
            .expect("search");
        let HistoryResp::Search { page } = response else {
            panic!("the wrong response was returned");
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].name, "Screenshot_alpha.png");
        let response = service
            .handle(HistoryReq::Search {
                text: "alpha".to_owned(),
                filter: HistoryFilter::Videos,
                cursor: None,
            })
            .expect("search");
        let HistoryResp::Search { page } = response else {
            panic!("the wrong response was returned");
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].kind, CaptureKindDto::Video);
    }

    #[test]
    fn delete_recycles_the_file_and_removes_the_row_and_thumbnail() {
        let fixture = fixture();
        let (id, path) = fixture.capture("gone.png", CaptureKind::Image, BASE);
        let thumbnail = fixture.paths.thumbs_dir.join(format!("{id}.webp"));
        std::fs::create_dir_all(&fixture.paths.thumbs_dir).expect("thumbs folder");
        std::fs::write(&thumbnail, b"thumb").expect("thumbnail");
        let service = fixture.service();
        let response = service.handle(HistoryReq::Delete { id }).expect("delete");
        assert_eq!(response, HistoryResp::Delete { id });
        assert_eq!(
            fixture.host.calls(),
            vec![format!("recycle:{}", path.display())]
        );
        assert!(!thumbnail.exists());
        assert_eq!(fixture.repo().get(id).expect("get"), None);
        assert_eq!(
            service.handle(HistoryReq::Delete { id }),
            Err(ServiceError::NotFound)
        );
    }

    #[test]
    fn a_failed_recycle_keeps_the_row_and_thumbnail() {
        let fixture = fixture();
        let (id, _path) = fixture.capture("kept.png", CaptureKind::Image, BASE);
        let thumbnail = fixture.paths.thumbs_dir.join(format!("{id}.webp"));
        std::fs::create_dir_all(&fixture.paths.thumbs_dir).expect("thumbs folder");
        std::fs::write(&thumbnail, b"thumb").expect("thumbnail");
        let host = Arc::new(FakeHost {
            fail_recycle: true,
            ..FakeHost::default()
        });
        let service = HistoryService::with_host(&fixture.services, host);
        assert!(service.handle(HistoryReq::Delete { id }).is_err());
        assert!(thumbnail.exists());
        assert!(fixture.repo().get(id).expect("get").is_some());
    }

    #[test]
    fn a_borrowed_file_is_revealed_and_remains_on_disk_after_history_deletion() {
        use snapline_store::history::{BorrowedFile, Origin};

        let fixture = fixture();
        let path = fixture._directory.path().join("original.exe");
        fs::write(&path, b"original").expect("original file");
        let id = fixture
            .repo()
            .record_borrowed_files(&[BorrowedFile {
                kind: CaptureKind::File,
                origin: Origin::File,
                path: path.to_string_lossy().into_owned(),
                bytes: 8,
                source_modified_ms: BASE,
                created_at: BASE,
            }])
            .expect("borrowed file")[0];
        let service = fixture.service();
        service
            .handle(HistoryReq::OpenFile { id })
            .expect("show original");
        service
            .handle(HistoryReq::Delete { id })
            .expect("remove row");
        assert_eq!(
            fixture.host.calls(),
            vec![format!("show:{}", path.display())]
        );
        assert_eq!(fs::read(&path).expect("original remains"), b"original");
        assert_eq!(fixture.repo().get(id).expect("lookup"), None);
    }

    #[test]
    fn confirm_delete_uses_the_capture_name() {
        let fixture = fixture();
        let (id, _path) = fixture.capture("named.png", CaptureKind::Image, BASE);
        let host = Arc::new(FakeHost {
            confirmed: true,
            ..FakeHost::default()
        });
        let service = HistoryService::with_host(&fixture.services, host.clone());
        let response = service
            .handle(HistoryReq::ConfirmDelete { id })
            .expect("confirm");
        assert_eq!(response, HistoryResp::ConfirmDelete { confirmed: true });
        assert_eq!(
            host.calls(),
            vec!["confirm:Delete named.png? The file goes to the Recycle Bin.".to_owned()]
        );
    }

    #[test]
    fn copy_image_reads_the_file_and_rejects_timed_captures() {
        let fixture = fixture();
        let (id, _path) = fixture.capture("copy.png", CaptureKind::Image, BASE);
        let service = fixture.service();
        assert_eq!(
            service.handle(HistoryReq::CopyImage { id }).expect("copy"),
            HistoryResp::CopyImage
        );
        assert_eq!(
            fixture.host.image.lock().expect("image").as_slice(),
            b"capture bytes"
        );
        let (video, _path) = fixture.capture("clip.mp4", CaptureKind::Video, BASE + 1);
        assert!(matches!(
            service.handle(HistoryReq::CopyImage { id: video }),
            Err(ServiceError::Invalid(_))
        ));
    }

    #[test]
    fn open_and_show_in_folder_use_the_stored_path() {
        let fixture = fixture();
        let (id, path) = fixture.capture("open.png", CaptureKind::Image, BASE);
        let service = fixture.service();
        assert_eq!(
            service.handle(HistoryReq::OpenFile { id }).expect("open"),
            HistoryResp::OpenFile
        );
        assert_eq!(
            service
                .handle(HistoryReq::ShowInFolder { id })
                .expect("show"),
            HistoryResp::ShowInFolder
        );
        assert_eq!(
            fixture.host.calls(),
            vec![
                format!("open:{}", path.display()),
                format!("show:{}", path.display())
            ]
        );
    }

    #[test]
    fn open_url_only_accepts_http_and_https_links() {
        let fixture = fixture();
        let (id, _path) = fixture.capture("link.png", CaptureKind::Image, BASE);
        fixture.upload(
            id,
            StoreUploadState::Done,
            Some("https://example.com/link.png"),
        );
        let service = fixture.service();
        assert_eq!(
            service.handle(HistoryReq::OpenUrl { id }).expect("url"),
            HistoryResp::OpenUrl
        );
        assert_eq!(
            fixture.host.calls(),
            vec!["url:https://example.com/link.png".to_owned()]
        );
        let (unsafe_id, _path) = fixture.capture("bad.png", CaptureKind::Image, BASE + 1);
        fixture.upload(
            unsafe_id,
            StoreUploadState::Done,
            Some("file:///C:/Windows/System32/calc.exe"),
        );
        assert!(matches!(
            service.handle(HistoryReq::OpenUrl { id: unsafe_id }),
            Err(ServiceError::Invalid(_))
        ));
        let (local_id, _path) = fixture.capture("local.png", CaptureKind::Image, BASE + 2);
        assert!(matches!(
            service.handle(HistoryReq::OpenUrl { id: local_id }),
            Err(ServiceError::Invalid(_))
        ));
    }
}
