use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use snapline_core::media::{classify, content_type, MediaKind};
use snapline_core::ports::Notice;
use snapline_store::history::{BorrowedFile, CaptureKind, HistoryRepo, Origin};
use snapline_store::settings::uploads::DestinationRef;
use snapline_upload::queue::{self, LinkBatch, UploadFile};

use crate::uploads::{destination_for, DestinationSource, UploadRuntime};
use crate::{ServiceError, Services};

static NEXT_BATCH: AtomicU64 = AtomicU64::new(1);

pub enum IntakeItem {
    File(PathBuf),
}

pub struct IntakeRequest {
    pub items: Vec<IntakeItem>,
    pub origin: Origin,
    pub dest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub capture_id: i64,
    pub upload_id: u64,
}

struct PreparedFile {
    path: PathBuf,
    file_name: String,
    media: MediaKind,
    len: u64,
    modified_ms: i64,
    destination: String,
    uploader: Arc<dyn queue::Uploader>,
}

pub struct Intake {
    services: Services,
    queue: queue::Handle,
    destinations: Arc<dyn DestinationSource>,
}

impl Intake {
    pub fn new(services: &Services, runtime: &UploadRuntime) -> Self {
        Self {
            services: services.clone(),
            queue: runtime.queue.clone(),
            destinations: Arc::clone(&runtime.destinations),
        }
    }

    pub fn pick_and_upload(&self) -> Result<Vec<Queued>, ServiceError> {
        let paths = self.services.pick_files()?;
        self.upload(IntakeRequest {
            items: paths.into_iter().map(IntakeItem::File).collect(),
            origin: Origin::File,
            dest: None,
        })
    }

    pub fn upload(&self, request: IntakeRequest) -> Result<Vec<Queued>, ServiceError> {
        if request.items.is_empty() {
            return Ok(Vec::new());
        }
        let settings = self.services.settings_snapshot()?;
        let mut files = Vec::with_capacity(request.items.len());
        for item in request.items {
            let IntakeItem::File(path) = item;
            let mut file =
                File::open(&path).map_err(|error| ServiceError::Io(error.to_string()))?;
            let metadata = file
                .metadata()
                .map_err(|error| ServiceError::Io(error.to_string()))?;
            if !metadata.is_file() {
                return Err(ServiceError::Invalid(
                    "Choose a regular file to upload.".to_owned(),
                ));
            }
            let len = metadata.len();
            if i64::try_from(len).is_err() {
                return Err(ServiceError::Invalid(
                    "The file is too large to add to History.".to_owned(),
                ));
            }
            let modified_ms = modified_ms(&metadata)?;
            let mut first_bytes = [0_u8; 512];
            let first_len = file
                .read(&mut first_bytes)
                .map_err(|error| ServiceError::Io(error.to_string()))?;
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("upload")
                .to_owned();
            let media = classify(&file_name, &first_bytes[..first_len]);
            let destination = request.dest.clone().unwrap_or_else(|| {
                destination_for(&settings.uploads, media)
                    .as_str()
                    .to_owned()
            });
            let setting = setting_for(media);
            if !DestinationRef::is_upload_id(&destination) {
                let message = format!("Choose an upload destination in Settings for {setting}.");
                if let Some(notifier) = self.services.notifier() {
                    notifier.toast(Notice::new("Upload not started", &message));
                }
                return Err(ServiceError::Invalid(message));
            }
            if !self.destinations.streams_files(&destination)
                && len > queue::MAX_BUFFERED_UPLOAD_BYTES
            {
                return Err(ServiceError::Invalid(format!(
                    "This destination can upload files up to {} MiB.",
                    queue::MAX_BUFFERED_UPLOAD_BYTES / (1024 * 1024)
                )));
            }
            let uploader = self.destinations.uploader(&destination).ok_or_else(|| {
                ServiceError::Invalid("The upload destination was not found.".to_owned())
            })?;
            files.push(PreparedFile {
                path,
                file_name,
                media,
                len,
                modified_ms,
                destination,
                uploader,
            });
        }
        let paths = self.services.paths()?;
        let mut history = HistoryRepo::open_write(&paths.history_file)
            .map_err(|error| ServiceError::Io(error.to_string()))?;
        let size = u32::try_from(files.len())
            .map_err(|_| ServiceError::Invalid("Too many files were selected.".to_owned()))?;
        let created_at = now_millis()?;
        let rows: Vec<_> = files
            .iter()
            .map(|file| BorrowedFile {
                kind: capture_kind(file.media),
                origin: request.origin,
                path: file.path.to_string_lossy().into_owned(),
                bytes: i64::try_from(file.len).unwrap_or(i64::MAX),
                source_modified_ms: file.modified_ms,
                created_at,
            })
            .collect();
        let capture_ids = history
            .record_borrowed_files(&rows)
            .map_err(|error| ServiceError::Io(error.to_string()))?;
        let batch_id = format!(
            "{}-{}",
            std::process::id(),
            NEXT_BATCH.fetch_add(1, Ordering::Relaxed)
        );
        let jobs = files
            .into_iter()
            .zip(capture_ids.iter().copied())
            .zip(0..size)
            .map(|((file, capture_id), index)| {
                let mime = content_type(&file.file_name);
                let upload_file = UploadFile::from_path(
                    file.path,
                    file.file_name,
                    mime,
                    file.len,
                    file.modified_ms,
                );
                queue::UploadJob::new(file.uploader, upload_file, file.destination, None)
                    .with_capture_id(capture_id)
                    .with_link_batch(LinkBatch {
                        id: batch_id.clone(),
                        index,
                        size,
                    })
            })
            .collect();
        let upload_ids = self.queue.enqueue_batch(jobs).map_err(|_| {
            ServiceError::Internal(
                "The files were added to History, but the upload queue is unavailable.".to_owned(),
            )
        })?;
        Ok(capture_ids
            .into_iter()
            .zip(upload_ids)
            .map(|(capture_id, upload_id)| Queued {
                capture_id,
                upload_id: upload_id.get(),
            })
            .collect())
    }
}

fn capture_kind(media: MediaKind) -> CaptureKind {
    match media {
        MediaKind::Image => CaptureKind::Image,
        MediaKind::Video => CaptureKind::Video,
        MediaKind::Gif => CaptureKind::Gif,
        MediaKind::Text => CaptureKind::Text,
        MediaKind::File => CaptureKind::File,
    }
}

fn setting_for(media: MediaKind) -> &'static str {
    match media {
        MediaKind::Image => "uploads.images",
        MediaKind::Video | MediaKind::Gif => "uploads.videos_and_gifs",
        MediaKind::Text => "uploads.text_and_code",
        MediaKind::File => "uploads.other_files",
    }
}

fn modified_ms(metadata: &std::fs::Metadata) -> Result<i64, ServiceError> {
    let elapsed = metadata
        .modified()
        .map_err(|error| ServiceError::Io(error.to_string()))?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ServiceError::Io(error.to_string()))?;
    i64::try_from(elapsed.as_millis()).map_err(|error| ServiceError::Io(error.to_string()))
}

fn now_millis() -> Result<i64, ServiceError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ServiceError::Internal(error.to_string()))?;
    i64::try_from(elapsed.as_millis()).map_err(|error| ServiceError::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::Duration;

    use snapline_store::history::{CaptureKind, HistoryRepo, Origin};
    use snapline_store::io::Paths;
    use snapline_upload::error::UploadError;
    use tempfile::TempDir;

    use super::*;
    use crate::uploads::{DestinationSource, FileSource, UploadRuntime};

    struct TestSecrets;

    impl queue::SecretStore for TestSecrets {
        fn get(
            &self,
            _destination: &str,
            _field: &str,
        ) -> Result<Option<Vec<u8>>, queue::SecretError> {
            Ok(None)
        }
    }

    struct TestUploader {
        uploaded: Sender<(String, String)>,
    }

    impl queue::Uploader for TestUploader {
        fn upload(
            &self,
            file: &UploadFile,
            _secrets: &dyn queue::SecretStore,
        ) -> Result<queue::Uploaded, UploadError> {
            let _ = self
                .uploaded
                .send((file.file_name.clone(), file.content_type.clone()));
            Ok(queue::Uploaded::new(format!(
                "https://example.test/{}",
                file.file_name
            )))
        }

        fn delete(
            &self,
            _link: &queue::DeletionLink,
            _secrets: &dyn queue::SecretStore,
        ) -> Result<(), UploadError> {
            Ok(())
        }
    }

    struct TestDestinations {
        uploader: Option<Arc<dyn queue::Uploader>>,
        streams_files: bool,
    }

    impl DestinationSource for TestDestinations {
        fn uploader(&self, destination: &str) -> Option<Arc<dyn queue::Uploader>> {
            destination
                .ends_with("-destination")
                .then(|| self.uploader.clone())
                .flatten()
        }

        fn streams_files(&self, _destination: &str) -> bool {
            self.streams_files
        }
    }

    struct NoFiles;

    impl FileSource for NoFiles {
        fn file(&self, _capture_id: i64) -> Result<Option<queue::UploadFile>, ServiceError> {
            Ok(None)
        }
    }

    struct Fixture {
        directory: TempDir,
        paths: Paths,
        services: Services,
        runtime: UploadRuntime,
        _queue: queue::Queue,
        uploaded: Receiver<(String, String)>,
    }

    impl Fixture {
        fn new(has_uploader: bool, streams_files: bool) -> Self {
            let directory = tempfile::Builder::new()
                .prefix("intake-test-")
                .tempdir_in(".")
                .expect("temporary directory");
            let root = directory.path();
            let paths = Paths::resolve(
                &root.join("app"),
                &root.join("roaming"),
                &root.join("local"),
                &root.join("profile"),
                false,
            );
            let services = Services::with_paths(paths.clone()).expect("services");
            services
                .update_settings(&serde_json::json!({
                    "uploads": {
                        "images": "image-destination",
                        "videos_and_gifs": "video-destination",
                        "text_and_code": "text-destination",
                        "other_files": "file-destination"
                    }
                }))
                .expect("upload settings");
            let (uploaded_tx, uploaded_rx) = mpsc::channel();
            let uploader: Arc<dyn queue::Uploader> = Arc::new(TestUploader {
                uploaded: uploaded_tx,
            });
            let destinations = Arc::new(TestDestinations {
                uploader: has_uploader.then_some(uploader),
                streams_files,
            });
            let queue = queue::Queue::start_with_sink(
                Arc::new(TestSecrets),
                queue::RetryPolicy::new(0),
                |_| {},
            )
            .expect("upload queue");
            let runtime = UploadRuntime {
                queue: queue.handle(),
                files: Arc::new(NoFiles),
                destinations,
            };
            Self {
                directory,
                paths,
                services,
                runtime,
                _queue: queue,
                uploaded: uploaded_rx,
            }
        }
    }

    #[test]
    fn uploads_mixed_files_with_matching_history_and_destination_metadata() {
        let fixture = Fixture::new(true, false);
        let samples = [
            (
                "photo.PNG",
                b"png data".as_slice(),
                CaptureKind::Image,
                "image-destination",
                "image/png",
            ),
            (
                "clip.mkv",
                b"video data".as_slice(),
                CaptureKind::Video,
                "video-destination",
                "video/x-matroska",
            ),
            (
                "motion.gif",
                b"gif data".as_slice(),
                CaptureKind::Gif,
                "video-destination",
                "image/gif",
            ),
            (
                "notes.txt",
                b"text data".as_slice(),
                CaptureKind::Text,
                "text-destination",
                "text/plain",
            ),
            (
                "archive.zip",
                b"zip data".as_slice(),
                CaptureKind::File,
                "file-destination",
                "application/zip",
            ),
        ];
        let paths: Vec<_> = samples
            .iter()
            .map(|(name, bytes, _, _, _)| {
                let path = fixture.directory.path().join(name);
                std::fs::write(&path, bytes).expect("source file");
                path
            })
            .collect();
        let intake = Intake::new(&fixture.services, &fixture.runtime);

        let queued = intake
            .upload(IntakeRequest {
                items: paths.iter().cloned().map(IntakeItem::File).collect(),
                origin: Origin::Watch,
                dest: None,
            })
            .expect("files queued");

        assert_eq!(queued.len(), samples.len());
        let history = HistoryRepo::open_read(&fixture.paths.history_file).expect("history");
        let mut expected_destinations = HashMap::new();
        for (queued_file, (name, bytes, kind, destination, _)) in queued.iter().zip(samples) {
            let capture = history
                .get(queued_file.capture_id)
                .expect("history lookup")
                .expect("history row");
            assert_eq!(capture.kind, kind);
            assert_eq!(capture.origin, Origin::Watch);
            assert!(!capture.owned);
            assert_eq!(
                capture.path,
                fixture.directory.path().join(name).to_string_lossy()
            );
            assert_eq!(
                capture.bytes,
                i64::try_from(bytes.len()).expect("file length")
            );
            assert!(capture.source_modified_ms.is_some());
            expected_destinations.insert(name.to_owned(), destination.to_owned());
        }

        let snapshots = fixture.runtime.queue.list().expect("queued uploads");
        assert_eq!(snapshots.len(), samples.len());
        for snapshot in snapshots {
            assert_eq!(snapshot.dest, expected_destinations[&snapshot.file_name]);
            assert_eq!(
                snapshot.link_batch.as_ref().map(|batch| batch.size),
                Some(5)
            );
        }

        let mut uploaded = Vec::new();
        for _ in 0..samples.len() {
            uploaded.push(
                fixture
                    .uploaded
                    .recv_timeout(Duration::from_secs(5))
                    .expect("uploader received file"),
            );
        }
        uploaded.sort();
        let mut expected_types: Vec<_> = samples
            .iter()
            .map(|(name, _, _, _, mime)| (name.to_string(), mime.to_string()))
            .collect();
        expected_types.sort();
        assert_eq!(uploaded, expected_types);
    }

    #[test]
    fn returns_empty_for_no_items_without_a_data_folder() {
        let fixture = Fixture::new(true, false);
        let intake = Intake::new(&Services::new(), &fixture.runtime);

        let queued = intake
            .upload(IntakeRequest {
                items: Vec::new(),
                origin: Origin::File,
                dest: None,
            })
            .expect("empty selection");

        assert!(queued.is_empty());
    }

    #[test]
    fn rejects_files_over_the_buffer_limit_for_non_streaming_destinations() {
        let fixture = Fixture::new(true, false);
        let path = fixture.directory.path().join("large.bin");
        let file = std::fs::File::create(&path).expect("large file");
        file.set_len(queue::MAX_BUFFERED_UPLOAD_BYTES + 1)
            .expect("sparse file length");
        let intake = Intake::new(&fixture.services, &fixture.runtime);

        let result = intake.upload(IntakeRequest {
            items: vec![IntakeItem::File(path)],
            origin: Origin::File,
            dest: Some("large-destination".to_owned()),
        });

        assert!(
            matches!(result, Err(ServiceError::Invalid(message)) if message.contains("64 MiB"))
        );
    }

    #[test]
    fn allows_large_files_when_the_destination_streams_them() {
        let fixture = Fixture::new(true, true);
        let path = fixture.directory.path().join("large.bin");
        let file = std::fs::File::create(&path).expect("large file");
        file.set_len(queue::MAX_BUFFERED_UPLOAD_BYTES + 1)
            .expect("sparse file length");
        let intake = Intake::new(&fixture.services, &fixture.runtime);

        let queued = intake
            .upload(IntakeRequest {
                items: vec![IntakeItem::File(path)],
                origin: Origin::File,
                dest: Some("stream-destination".to_owned()),
            })
            .expect("streaming upload queued");

        assert_eq!(queued.len(), 1);
        let history = HistoryRepo::open_read(&fixture.paths.history_file).expect("history");
        let capture = history
            .get(queued[0].capture_id)
            .expect("history lookup")
            .expect("history row");
        assert_eq!(
            capture.bytes,
            i64::try_from(queue::MAX_BUFFERED_UPLOAD_BYTES + 1).unwrap()
        );
        assert_eq!(
            fixture
                .uploaded
                .recv_timeout(Duration::from_secs(5))
                .expect("uploader received file"),
            (
                "large.bin".to_owned(),
                "application/octet-stream".to_owned()
            )
        );
    }

    #[test]
    fn explains_which_destination_setting_is_missing_for_each_media_kind() {
        let fixture = Fixture::new(true, false);
        let services = Services::new();
        let intake = Intake::new(&services, &fixture.runtime);
        let cases = [
            ("photo.png", "uploads.images"),
            ("clip.mp4", "uploads.videos_and_gifs"),
            ("motion.gif", "uploads.videos_and_gifs"),
            ("notes.txt", "uploads.text_and_code"),
            ("archive.zip", "uploads.other_files"),
        ];

        for (name, setting) in cases {
            let path = fixture.directory.path().join(name);
            std::fs::write(&path, b"content").expect("source file");
            let result = intake.upload(IntakeRequest {
                items: vec![IntakeItem::File(path)],
                origin: Origin::File,
                dest: None,
            });

            assert!(
                matches!(result, Err(ServiceError::Invalid(message)) if message.contains(setting)),
                "{name}"
            );
        }
    }

    #[test]
    fn rejects_a_configured_destination_that_has_no_uploader() {
        let fixture = Fixture::new(false, false);
        let path = fixture.directory.path().join("photo.png");
        std::fs::write(&path, b"png data").expect("source file");
        let intake = Intake::new(&fixture.services, &fixture.runtime);

        let result = intake.upload(IntakeRequest {
            items: vec![IntakeItem::File(path)],
            origin: Origin::File,
            dest: None,
        });

        assert_eq!(
            result,
            Err(ServiceError::Invalid(
                "The upload destination was not found.".to_owned()
            ))
        );
    }
}
