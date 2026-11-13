use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use snapline_core::image::{Image, ImageFormat};
use snapline_core::naming::{self, NameContext};
use snapline_store::history::{CaptureKind, HistoryRepo, NewCapture};
use snapline_store::io::{self, Paths};
use snapline_store::settings::general::{General, Subfolders};
use time::OffsetDateTime;

use crate::saver::{self, THUMBNAIL_WIDTH};
use crate::Services;

use super::{remove_file, RecordError, RecordedFile, RecordedKind, RecordingStore, SavedRecording};

pub struct ServicesRecordingStore {
    services: Services,
}

impl ServicesRecordingStore {
    pub fn new(services: &Services) -> Self {
        Self {
            services: services.clone(),
        }
    }
}

impl RecordingStore for ServicesRecordingStore {
    fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError> {
        store_recording(&self.services, file)
    }
}

#[derive(Clone)]
pub struct WorkerRecordingStore {
    worker: crate::worker::Handle,
}

impl WorkerRecordingStore {
    pub fn new(worker: crate::worker::Handle) -> Self {
        Self { worker }
    }
}

impl RecordingStore for WorkerRecordingStore {
    fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError> {
        self.worker
            .save_recording(file)
            .map_err(|error| RecordError::Save(error.to_string()))
    }
}

pub fn store_recording(
    services: &Services,
    file: RecordedFile,
) -> Result<SavedRecording, RecordError> {
    let settings = services
        .settings_snapshot()
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let paths = services
        .paths()
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let general = &settings.general;
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let folder = capture_folder(general, &paths, now);
    std::fs::create_dir_all(&folder).map_err(|error| RecordError::Save(error.to_string()))?;
    let counter = saver::counter_from(general);
    let context = NameContext {
        app: file.source_app.clone(),
        window: file.window_title.clone(),
        timestamp: now,
        counter,
        width: file.width,
        height: file.height,
    };
    let extension = file
        .media
        .extension()
        .and_then(OsStr::to_str)
        .filter(|extension| !extension.is_empty())
        .unwrap_or("mp4");
    let stem = naming::expand(&general.name_pattern, &context, &naming::ThreadRandom);
    let name = naming::sanitise_file_name(&format!("{stem}.{extension}"));
    let file_name = naming::resolve_collision(&name, |candidate| folder.join(candidate).exists());
    let path = folder.join(&file_name);
    move_file(&file.media, &path)?;
    let bytes = std::fs::metadata(&path)
        .map_err(|error| RecordError::Save(error.to_string()))?
        .len();
    let bytes =
        i64::try_from(bytes).map_err(|_| RecordError::Save("The file is too large.".to_owned()))?;
    let created_at = now.unix_timestamp().saturating_mul(1000) + i64::from(now.millisecond());
    let kind = match file.kind {
        RecordedKind::Video => CaptureKind::Video,
        RecordedKind::Gif => CaptureKind::Gif,
    };
    let mut repo = HistoryRepo::open_write(&paths.history_file)
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let id = repo
        .record(NewCapture {
            kind,
            path: path.to_string_lossy().into_owned(),
            bytes,
            width: Some(i64::from(file.width)),
            height: Some(i64::from(file.height)),
            duration_ms: Some(i64::try_from(file.duration_ms).unwrap_or(i64::MAX)),
            source_app: file.source_app.clone(),
            created_at,
            ocr_text: None,
        })
        .map_err(|error| RecordError::Save(error.to_string()))?;
    if let Some(frame) = &file.first_frame {
        if let Err(error) = write_thumbnail(
            services,
            &paths.thumbs_dir,
            id,
            frame,
            general.lossy_quality,
        ) {
            tracing::warn!(%error, "the recording thumbnail could not be written");
        }
    }
    let next_counter = naming::next_counter(counter);
    if let Err(error) = services.update_settings(&serde_json::json!({
        "general": { "counter": i64::from(next_counter) }
    })) {
        tracing::warn!(%error, "the capture counter could not be saved");
    }
    Ok(SavedRecording {
        capture_id: Some(id),
        path,
        file_name,
        bytes,
        width: file.width,
        height: file.height,
        duration_ms: file.duration_ms,
        kind: file.kind,
    })
}

fn capture_folder(general: &General, paths: &Paths, now: OffsetDateTime) -> PathBuf {
    let profile = paths
        .captures_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""));
    let mut folder = PathBuf::from(saver::expand_profile(&general.save_folder, profile));
    match general.subfolders {
        Subfolders::ByMonth => {
            folder.push(format!("{:04}-{:02}", now.year(), now.month() as u8));
        }
        Subfolders::ByDay => folder.push(format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            now.month() as u8,
            now.day()
        )),
        Subfolders::ByApp => folder.push(naming::sanitise_file_name(&naming::app_label(None))),
        Subfolders::None => {}
    }
    folder
}

fn move_file(from: &Path, to: &Path) -> Result<(), RecordError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|error| RecordError::Save(error.to_string()))?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|error| RecordError::Save(error.to_string()))?;
    remove_file(from);
    Ok(())
}

fn write_thumbnail(
    services: &Services,
    directory: &Path,
    id: i64,
    image: &Image,
    quality: u8,
) -> Result<(), RecordError> {
    let codec = services
        .codec()
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let thumbnail = saver::scale_to_width(image, THUMBNAIL_WIDTH)
        .map_err(|error| RecordError::Save(error.to_string()))?;
    let quality = quality.clamp(1, 100);
    let (bytes, extension) = match codec.encode(&thumbnail, ImageFormat::WebP, quality) {
        Ok(bytes) => (bytes, "webp"),
        Err(_) => (
            codec
                .encode(&thumbnail, ImageFormat::Jpeg, quality)
                .map_err(|error| RecordError::Save(error.to_string()))?,
            "jpg",
        ),
    };
    std::fs::create_dir_all(directory).map_err(|error| RecordError::Save(error.to_string()))?;
    io::write_atomic(&directory.join(format!("{id}.{extension}")), &bytes)
        .map_err(|error| RecordError::Save(error.to_string()))
}
