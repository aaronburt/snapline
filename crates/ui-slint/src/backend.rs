use std::io::Cursor as IoCursor;
use std::sync::Arc;

use anyhow::{bail, Context};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageEncoder, ImageFormat as RustImageFormat, RgbImage, RgbaImage};
use serde_json::{Map, Value};
use slint::{Rgba8Pixel, SharedPixelBuffer};
use snapline_core::image::{Image, ImageFormat};
use snapline_core::ports::{CodecError, ImageCodec, SecretStore};
use snapline_services::destinations::Registry;
use snapline_services::dto::app::{AppReq, AppResp};
use snapline_services::dto::history::{
    CaptureDto, Cursor, HistoryFilter, HistoryPage, HistoryReq, HistoryResp,
};
use snapline_services::dto::hotkeys::{HotkeysReq, HotkeysResp};
use snapline_services::dto::settings::{SettingsDto, SettingsReq, SettingsResp};
use snapline_services::dto::tool::{CombineRequestDto, ToolReq, ToolResp};
use snapline_services::dto::upload::{
    DestinationKindDto, DestinationReq, DestinationResp, DestinationViewDto, UploadReq, UploadResp,
    UploadState,
};
use snapline_services::dto::workflow::{WorkflowReq, WorkflowResp};
use snapline_services::uploads::{retry_policy, DestinationSource, UploaderThread};
use snapline_services::{ServiceError, Services};
use snapline_store::history::HistoryRepo;
use snapline_store::io::Paths;

use crate::host::{Host, PlainHost, QrScanResult};
use crate::secrets::MemorySecrets;
use crate::upload_files::HistoryFileSource;

pub struct RasterCodec;

impl ImageCodec for RasterCodec {
    fn encode(
        &self,
        image: &Image,
        format: ImageFormat,
        quality: u8,
    ) -> Result<Vec<u8>, CodecError> {
        let width = image.width();
        let height = image.height();
        let mut cursor = IoCursor::new(Vec::new());
        match format {
            ImageFormat::Jpeg => {
                let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
                for y in 0..height {
                    let row = image
                        .row(y)
                        .ok_or_else(|| CodecError::Encode("image row out of bounds".to_owned()))?;
                    for px in row.chunks_exact(4) {
                        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
                    }
                }
                let buffer = RgbImage::from_raw(width, height, rgb)
                    .ok_or_else(|| CodecError::Encode("invalid JPEG image buffer".to_owned()))?;
                let q = if quality == 0 {
                    90
                } else {
                    quality.clamp(1, 100)
                };
                JpegEncoder::new_with_quality(&mut cursor, q)
                    .write_image(
                        buffer.as_raw(),
                        width,
                        height,
                        image::ColorType::Rgb8.into(),
                    )
                    .map_err(|error| CodecError::Encode(error.to_string()))?;
            }
            ImageFormat::Png | ImageFormat::WebP => {
                let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                for y in 0..height {
                    let row = image
                        .row(y)
                        .ok_or_else(|| CodecError::Encode("image row out of bounds".to_owned()))?;
                    for px in row.chunks_exact(4) {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                    }
                }
                let buffer = RgbaImage::from_raw(width, height, rgba)
                    .ok_or_else(|| CodecError::Encode("invalid RGBA image buffer".to_owned()))?;
                let target = match format {
                    ImageFormat::Png => RustImageFormat::Png,
                    ImageFormat::WebP => RustImageFormat::WebP,
                    ImageFormat::Jpeg => RustImageFormat::Jpeg,
                };
                DynamicImage::ImageRgba8(buffer)
                    .write_to(&mut cursor, target)
                    .map_err(|error| CodecError::Encode(error.to_string()))?;
            }
        }
        Ok(cursor.into_inner())
    }

    fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError> {
        let rgba = image::load_from_memory(bytes)
            .map_err(|error| CodecError::Decode(error.to_string()))?
            .to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
        for px in rgba.as_raw().chunks_exact(4) {
            bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        Image::new(width, height, width as usize * 4, bgra)
            .map_err(|error| CodecError::Decode(error.to_string()))
    }
}

pub fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((width, height, rgba.into_raw()))
}

pub fn rgba_to_slint(width: u32, height: u32, rgba: &[u8]) -> slint::Image {
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(rgba, width, height);
    slint::Image::from_rgba8(buffer)
}

pub fn decode_to_slint(bytes: &[u8]) -> Option<slint::Image> {
    let (width, height, rgba) = decode_rgba(bytes)?;
    Some(rgba_to_slint(width, height, &rgba))
}

pub fn image_to_slint(image: &Image) -> Option<slint::Image> {
    let width = image.width();
    let height = image.height();
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let dest = buffer.make_mut_bytes();
    for y in 0..height {
        let row = image.row(y)?;
        let out_start = y as usize * width as usize * 4;
        for (x, px) in row.chunks_exact(4).enumerate() {
            let off = out_start + x * 4;
            dest[off] = px[2];
            dest[off + 1] = px[1];
            dest[off + 2] = px[0];
            dest[off + 3] = px[3];
        }
    }
    Some(slint::Image::from_rgba8(buffer))
}

pub struct Backend {
    services: Services,
    host: Arc<dyn Host>,
    _uploader: Option<UploaderThread>,
}

impl Backend {
    pub fn new(services: Services, host: Arc<dyn Host>) -> Self {
        Self {
            services,
            host,
            _uploader: None,
        }
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }
}

pub(crate) fn report_failure(backend: &Backend, what: &str, error: &anyhow::Error) {
    let detail = format!("{error:#}");
    tracing::error!(%what, error = %detail, "a Snapline window action failed");
    backend.host().report_error(what, &detail);
}

/// What a background job reports when it panics, so the page that started it can clear its busy state.
pub(crate) trait FromPanic {
    fn from_panic(message: String) -> Self;
}

impl<X> FromPanic for anyhow::Result<X> {
    fn from_panic(message: String) -> Self {
        Err(anyhow::anyhow!(message))
    }
}

impl<X: FromPanic> FromPanic for Option<X> {
    fn from_panic(message: String) -> Self {
        Some(X::from_panic(message))
    }
}

impl<A: FromPanic, B: Default> FromPanic for (A, B) {
    fn from_panic(message: String) -> Self {
        (A::from_panic(message), B::default())
    }
}

impl FromPanic for (Vec<CaptureDto>, Vec<CaptureDto>) {
    fn from_panic(_: String) -> Self {
        (Vec::new(), Vec::new())
    }
}

impl FromPanic for (Option<Vec<DestinationViewDto>>, Option<String>) {
    fn from_panic(_: String) -> Self {
        (None, None)
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned());
    match detail {
        Some(detail) => format!("Something went wrong in Snapline: {detail}"),
        None => "Something went wrong in Snapline.".to_owned(),
    }
}

pub(crate) fn spawn_bg<T, W, A>(backend: &Backend, work: W, apply: A)
where
    T: FromPanic + Send + 'static,
    W: FnOnce(Backend) -> T + Send + 'static,
    A: FnOnce(Backend, T) + Send + 'static,
{
    let services = backend.services.clone();
    let host = backend.host.clone();
    std::thread::spawn(move || {
        let bg = Backend::new(services.clone(), host.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(bg)))
            .unwrap_or_else(|payload| {
                let message = panic_message(payload.as_ref());
                tracing::error!(%message, "a background job in the Snapline window panicked");
                T::from_panic(message)
            });
        let _ = slint::invoke_from_event_loop(move || {
            let ui_backend = Backend::new(services, host);
            apply(ui_backend, result);
        });
    });
}

impl Backend {
    pub fn open() -> anyhow::Result<Self> {
        let paths = Paths::discover().context("locating Snapline folders")?;
        let services = Services::with_paths(paths).context("loading settings")?;
        let history_file = services.paths()?.history_file;
        let settings = services.settings_snapshot()?;

        let secrets: Arc<dyn SecretStore + Send + Sync> = Arc::new(MemorySecrets::default());
        let registry: Arc<dyn DestinationSource> =
            Arc::new(Registry::new(&services, secrets.clone()));
        let files = Arc::new(HistoryFileSource::new(history_file));
        let uploader = UploaderThread::start(
            &services,
            secrets,
            retry_policy(settings.uploads.retry),
            files,
            registry,
        )?;
        let services = services
            .with_codec_factory(Arc::new(
                || Ok(Arc::new(RasterCodec) as Arc<dyn ImageCodec>),
            ))
            .with_upload_runtime(uploader.runtime());
        Ok(Self {
            services,
            host: Arc::new(PlainHost),
            _uploader: Some(uploader),
        })
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn settings(&self) -> anyhow::Result<SettingsDto> {
        match self.services.settings().handle(SettingsReq::Get)? {
            SettingsResp::Settings { settings } => Ok(settings),
            other => bail!("unexpected settings response: {other:?}"),
        }
    }

    pub fn update(&self, patch: Value) -> anyhow::Result<SettingsDto> {
        self.host
            .update_settings(&self.services, patch)
            .map_err(anyhow::Error::msg)
    }

    pub fn pick_ffmpeg(&self) -> anyhow::Result<()> {
        self.services.settings().handle(SettingsReq::PickFfmpeg)?;
        Ok(())
    }

    pub fn reset_ffmpeg(&self) -> anyhow::Result<()> {
        self.services
            .settings()
            .handle(SettingsReq::ResetFfmpegPath)?;
        Ok(())
    }

    pub fn preview_name(&self, pattern: &str, format: &str) -> anyhow::Result<String> {
        match self.services.settings().handle(SettingsReq::PreviewName {
            pattern: pattern.to_string(),
            format: format.to_string(),
        })? {
            SettingsResp::PreviewName { name } => Ok(name),
            other => bail!("unexpected preview response: {other:?}"),
        }
    }

    pub fn hotkeys(&self) -> anyhow::Result<HotkeysResp> {
        Ok(self.services.hotkeys().handle(HotkeysReq::Get)?)
    }

    pub fn app_request(&self, req: AppReq) -> anyhow::Result<AppResp> {
        Ok(self.services.app().handle(req)?)
    }

    pub fn history_count(&self) -> anyhow::Result<i64> {
        match self.app_request(AppReq::Stats)? {
            AppResp::Stats { count } => Ok(count),
            other => bail!("unexpected app stats response: {other:?}"),
        }
    }

    pub fn tool_request(&self, req: ToolReq) -> anyhow::Result<ToolResp> {
        Ok(self.services.tool().handle(req)?)
    }

    pub fn upload_request(&self, req: UploadReq) -> anyhow::Result<UploadResp> {
        Ok(self.services.upload().handle(req)?)
    }

    pub fn run_action(&self, action: &str) -> anyhow::Result<()> {
        self.host.run_action(action).map_err(anyhow::Error::msg)
    }
}

pub fn patch(path: &str, value: Value) -> Value {
    path.split('.').rev().fold(value, |inner, key| {
        let mut object = Map::new();
        object.insert(key.to_string(), inner);
        Value::Object(object)
    })
}

impl Backend {
    pub fn destinations(&self) -> anyhow::Result<Vec<DestinationViewDto>> {
        match self.destination_request(DestinationReq::List)? {
            DestinationResp::Destinations(list) => Ok(list),
            other => bail!("unexpected destination response: {other:?}"),
        }
    }

    pub fn destination_request(&self, req: DestinationReq) -> anyhow::Result<DestinationResp> {
        match self.services.upload().handle(UploadReq::Destination(req))? {
            UploadResp::Destination(resp) => Ok(resp),
            other => bail!("unexpected upload response: {other:?}"),
        }
    }
}

pub fn destination_kind(kind: &str) -> anyhow::Result<DestinationKindDto> {
    serde_json::from_value(serde_json::json!(kind)).context("unknown destination kind")
}

impl Backend {
    pub fn workflow_request(&self, req: WorkflowReq) -> anyhow::Result<WorkflowResp> {
        Ok(self.services.workflow().handle(req)?)
    }
}

impl Backend {
    pub fn hotkey_preview(
        &self,
        owner: &str,
        chord: &str,
    ) -> anyhow::Result<snapline_services::dto::hotkeys::PreviewDto> {
        match self.services.hotkeys().handle(HotkeysReq::Preview {
            owner: owner.to_string(),
            chord: chord.to_string(),
        })? {
            HotkeysResp::Preview { preview } => Ok(preview),
            other => bail!("unexpected hotkey preview response: {other:?}"),
        }
    }
}

impl Backend {
    pub fn history_request(&self, req: HistoryReq) -> anyhow::Result<HistoryResp> {
        Ok(self.services.history().handle(req)?)
    }

    pub fn history_page_dto(
        &self,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
    ) -> anyhow::Result<HistoryPage> {
        match self.history_request(HistoryReq::Page { filter, cursor })? {
            HistoryResp::Page { page } => Ok(page),
            other => bail!("unexpected history page response: {other:?}"),
        }
    }

    pub fn history_search_dto(
        &self,
        text: &str,
        filter: HistoryFilter,
        cursor: Option<Cursor>,
    ) -> anyhow::Result<HistoryPage> {
        match self.history_request(HistoryReq::Search {
            text: text.to_string(),
            filter,
            cursor,
        })? {
            HistoryResp::Search { page } => Ok(page),
            other => bail!("unexpected history search response: {other:?}"),
        }
    }

    pub fn upload_states(&self) -> anyhow::Result<Vec<UploadState>> {
        match self.upload_request(UploadReq::List)? {
            UploadResp::States(states) => Ok(states),
            other => bail!("unexpected upload states response: {other:?}"),
        }
    }

    pub fn thumbnail(&self, id: i64) -> anyhow::Result<(Vec<u8>, &'static str)> {
        let thumb = self.services.thumbs().get(id)?;
        Ok((thumb.bytes, thumb.mime))
    }

    pub fn qr_make_preview(&self, text: &str) -> anyhow::Result<(slint::Image, Option<String>)> {
        let image = snapline_core::qr::make_image(text)?;
        let link = snapline_core::qr::link_of(text);
        let preview = image_to_slint(&image).context("invalid QR preview image")?;
        Ok((preview, link))
    }

    pub fn qr_png_bytes(&self, text: &str) -> anyhow::Result<Vec<u8>> {
        let image = snapline_core::qr::make_image(text)?;
        Ok(RasterCodec.encode(&image, ImageFormat::Png, 0)?)
    }

    pub fn copy_qr_image(&self, text: &str) -> anyhow::Result<()> {
        match self.services.tool().handle(ToolReq::QrCopyImage {
            text: text.to_owned(),
        }) {
            Ok(_) => return Ok(()),
            Err(ServiceError::NotYet) => {}
            Err(error) => return Err(error.into()),
        }
        let bytes = self.qr_png_bytes(text)?;
        self.host
            .copy_image_bytes(&bytes)
            .map_err(anyhow::Error::msg)
    }

    pub fn save_qr_png(&self, text: &str) -> anyhow::Result<Option<String>> {
        let bytes = self.qr_png_bytes(text)?;
        self.host
            .save_png("qr-code.png", &bytes)
            .map_err(anyhow::Error::msg)
    }

    pub fn open_qr_link(&self, link: &str) -> anyhow::Result<()> {
        match self.services.tool().handle(ToolReq::QrOpen {
            link: link.to_owned(),
        }) {
            Ok(_) => return Ok(()),
            Err(ServiceError::NotYet) => {}
            Err(error) => return Err(error.into()),
        }
        let clean =
            snapline_core::qr::link_of(link).context("Only http and https links can be opened.")?;
        self.host.open_url(&clean).map_err(anyhow::Error::msg)
    }

    pub fn scan_qr_screen(&self) -> anyhow::Result<Option<QrScanResult>> {
        let Some(image) = self
            .host
            .select_region_image()
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(None);
        };
        let scanned = snapline_services::tools::qr::scan(&image)?;
        Ok(Some(QrScanResult {
            is_url: scanned.is_url(),
            text: scanned.text,
            link: scanned.link,
        }))
    }

    pub fn scan_qr_capture(&self, id: i64) -> anyhow::Result<QrScanResult> {
        let paths = self.services.paths()?;
        if !paths.history_file.is_file() {
            bail!("The requested capture was not found.");
        }
        let repo = HistoryRepo::open_read(&paths.history_file)?;
        let capture = repo
            .get(id)?
            .context("The requested capture was not found.")?;
        let bytes = std::fs::read(&capture.path).context("The capture file could not be read.")?;
        let image = RasterCodec.decode(&bytes)?;
        let scanned = snapline_services::tools::qr::scan(&image)?;
        Ok(QrScanResult {
            is_url: scanned.is_url(),
            text: scanned.text,
            link: scanned.link,
        })
    }

    pub fn combine_preview_rgba(
        &self,
        request: CombineRequestDto,
    ) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let job = match self.tool_request(ToolReq::CombinePreview { request })? {
            ToolResp::CombinePreviewed { job } => job,
            other => bail!("unexpected combine preview response: {other:?}"),
        };
        let preview = snapline_services::tools::combine::preview(job)
            .context("the combined preview image is unavailable")?;
        decode_rgba(&preview.bytes).context("the combined preview image could not be decoded")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_results_become_errors_the_page_can_show() {
        let result: anyhow::Result<()> = FromPanic::from_panic("boom".to_owned());
        assert!(matches!(result, Err(error) if error.to_string() == "boom"));
        let picked: Option<anyhow::Result<()>> = FromPanic::from_panic("boom".to_owned());
        assert!(matches!(picked, Some(Err(_))));
        let pair: (anyhow::Result<()>, bool) = FromPanic::from_panic("boom".to_owned());
        assert!(pair.0.is_err());
        assert!(!pair.1);
    }

    #[test]
    fn panic_message_reads_str_and_string_payloads() {
        let text: Box<dyn std::any::Any + Send> = Box::new("bad state");
        assert_eq!(
            panic_message(text.as_ref()),
            "Something went wrong in Snapline: bad state"
        );
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("worse"));
        assert_eq!(
            panic_message(owned.as_ref()),
            "Something went wrong in Snapline: worse"
        );
        let other: Box<dyn std::any::Any + Send> = Box::new(7_u8);
        assert_eq!(
            panic_message(other.as_ref()),
            "Something went wrong in Snapline."
        );
    }

    #[test]
    fn patch_nests_dotted_paths() {
        assert_eq!(
            patch("general.image_format", serde_json::json!("png")),
            serde_json::json!({ "general": { "image_format": "png" } })
        );
        assert_eq!(
            patch("single", serde_json::json!(true)),
            serde_json::json!({ "single": true })
        );
    }

    #[test]
    fn destination_kind_parses_known_kinds_and_rejects_unknown() {
        assert_eq!(destination_kind("s3").ok(), Some(DestinationKindDto::S3));
        assert_eq!(
            destination_kind("gist").ok(),
            Some(DestinationKindDto::Gist)
        );
        assert_eq!(
            destination_kind("sftp").ok(),
            Some(DestinationKindDto::Sftp)
        );
        assert_eq!(
            destination_kind("custom").ok(),
            Some(DestinationKindDto::Custom)
        );
        assert!(destination_kind("bogus").is_err());
    }
}
