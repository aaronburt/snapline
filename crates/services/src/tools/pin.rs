use std::sync::{Arc, OnceLock};

use crate::dto::tool::{PinSourceDto, ToolResp};
use crate::editor::{EditorService, PinOutcome, PinSource, TOOL_EDITOR};
use crate::{ServiceError, Services};

pub const TOOL_PIN: &str = "tool_pin";

pub trait PinMenu: Send + Sync {
    fn choose(&self) -> Result<Option<PinSource>, ServiceError>;
}

static MENU: OnceLock<Arc<dyn PinMenu>> = OnceLock::new();

pub fn install_menu(menu: Arc<dyn PinMenu>) -> Result<(), ServiceError> {
    MENU.set(menu)
        .map_err(|_| ServiceError::Internal("The pin menu is already available.".to_owned()))
}

pub struct PinService {
    editor: EditorService,
}

impl PinService {
    pub fn new(services: &Services) -> Self {
        Self {
            editor: EditorService::new(services),
        }
    }

    pub fn with_editor(editor: EditorService) -> Self {
        Self { editor }
    }

    pub fn available(&self) -> bool {
        self.editor.pin_available()
    }

    pub fn history(&self, id: i64) -> Result<ToolResp, ServiceError> {
        Ok(response(self.editor.pin_history(id)?))
    }

    pub fn file(&self) -> Result<ToolResp, ServiceError> {
        Ok(response(self.editor.pin_file()?))
    }

    pub fn clipboard(&self) -> Result<ToolResp, ServiceError> {
        Ok(response(self.editor.pin_clipboard()?))
    }

    pub fn menu(&self) -> Result<ToolResp, ServiceError> {
        let menu = MENU.get().ok_or(ServiceError::NotYet)?;
        match menu.choose()? {
            Some(PinSource::File) => self.file(),
            Some(PinSource::Clipboard) => self.clipboard(),
            None => Ok(ToolResp::PinCancelled),
        }
    }
}

pub fn available(editor: &EditorService, tool: &str) -> bool {
    match tool {
        TOOL_EDITOR => editor.available(),
        TOOL_PIN => editor.pin_available(),
        _ => false,
    }
}

fn response(outcome: PinOutcome) -> ToolResp {
    match outcome {
        PinOutcome::Pinned(PinSource::File) => ToolResp::Pinned {
            source: PinSourceDto::File,
        },
        PinOutcome::Pinned(PinSource::Clipboard) => ToolResp::Pinned {
            source: PinSourceDto::Clipboard,
        },
        PinOutcome::Cancelled => ToolResp::PinCancelled,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use snapline_core::image::{Image, ImageFormat};
    use snapline_core::ports::{Clipboard, ClipboardError, CodecError, ImageCodec};
    use snapline_store::io::Paths;
    use tempfile::TempDir;

    use super::*;
    use crate::dto::tool::ToolReq;
    use crate::editor::{
        EditorClipboardContent, EditorClipboardSource, EditorFileDialog, EditorHost, EditorPin,
        EditorPorts, EditorResult, EditorSaver, EditorUploader,
    };
    use crate::saver::SaveOutcome;

    struct StubCodec;

    impl ImageCodec for StubCodec {
        fn encode(
            &self,
            _image: &Image,
            _format: ImageFormat,
            _quality: u8,
        ) -> Result<Vec<u8>, CodecError> {
            Ok(vec![1, 2, 3, 4])
        }

        fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError> {
            let (width, height) = match bytes.first() {
                Some(b'a') => (100, 50),
                Some(b'b') => (40, 80),
                Some(b'c') => (20, 10),
                _ => return Err(CodecError::Decode("unknown test image".to_owned())),
            };
            Image::bgra(width, height).map_err(|error| CodecError::Decode(error.to_string()))
        }
    }

    struct StubHost;

    impl EditorHost for StubHost {
        fn open(&self, _image: Image) -> Result<EditorResult, ServiceError> {
            Ok(EditorResult::cancelled())
        }
    }

    struct StubClipboard;

    impl Clipboard for StubClipboard {
        fn set_image(&self, _image: &Image) -> Result<(), ClipboardError> {
            Ok(())
        }

        fn set_text(&self, _text: &str) -> Result<(), ClipboardError> {
            Ok(())
        }

        fn get_image(&self) -> Result<Option<Image>, ClipboardError> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct StubDialog {
        path: Mutex<Option<PathBuf>>,
    }

    impl EditorFileDialog for StubDialog {
        fn pick_image(&self) -> Result<Option<PathBuf>, ServiceError> {
            Ok(self.path.lock().expect("path").clone())
        }
    }

    #[derive(Default)]
    struct StubSource {
        content: Mutex<Option<EditorClipboardContent>>,
    }

    impl EditorClipboardSource for StubSource {
        fn read(&self) -> Result<Option<EditorClipboardContent>, ServiceError> {
            Ok(self.content.lock().expect("content").clone())
        }
    }

    struct StubSaver;

    impl EditorSaver for StubSaver {
        fn save(&self, _image: Image, _title: String) -> Result<SaveOutcome, ServiceError> {
            Err(ServiceError::NotYet)
        }
    }

    struct StubUploader;

    impl EditorUploader for StubUploader {
        fn enqueue(&self, _capture_id: i64, _destination: &str) -> Result<u64, ServiceError> {
            Err(ServiceError::NotYet)
        }
    }

    #[derive(Default)]
    struct StubPin {
        images: Mutex<Vec<(u32, u32)>>,
    }

    impl EditorPin for StubPin {
        fn pin(&self, image: Image) -> Result<(), ServiceError> {
            self.images
                .lock()
                .expect("images")
                .push((image.width(), image.height()));
            Ok(())
        }
    }

    struct Ports {
        dialog: Arc<StubDialog>,
        source: Arc<StubSource>,
        pin: Arc<StubPin>,
    }

    impl Default for Ports {
        fn default() -> Self {
            Self {
                dialog: Arc::new(StubDialog::default()),
                source: Arc::new(StubSource::default()),
                pin: Arc::new(StubPin::default()),
            }
        }
    }

    struct Fixture {
        _directory: TempDir,
        services: Services,
        paths: Paths,
    }

    fn fixture() -> Fixture {
        let directory = tempfile::tempdir().expect("temp");
        let paths = Paths::resolve(
            &directory.path().join("exe"),
            &directory.path().join("roaming"),
            &directory.path().join("local"),
            &directory.path().join("profile"),
            false,
        );
        std::fs::create_dir_all(&paths.captures_dir).expect("captures");
        let services = Services::with_paths(paths.clone())
            .expect("services")
            .with_codec_factory(Arc::new(|| Ok(Arc::new(StubCodec) as Arc<dyn ImageCodec>)));
        Fixture {
            _directory: directory,
            services,
            paths,
        }
    }

    fn editor(fixture: &Fixture, ports: &Ports) -> EditorService {
        EditorService::with_ports(
            &fixture.services,
            EditorPorts {
                host: Arc::new(StubHost),
                file_dialog: ports.dialog.clone(),
                clipboard: Arc::new(StubClipboard),
                clipboard_source: ports.source.clone(),
                saver: Arc::new(StubSaver),
                uploader: Arc::new(StubUploader),
                pin: ports.pin.clone(),
            },
        )
    }

    fn image(width: u32, height: u32) -> Image {
        Image::bgra(width, height).expect("image")
    }

    #[test]
    fn the_pin_tool_is_available_only_when_the_ports_are_installed() {
        let fixture = fixture();
        let missing = EditorService::new(&fixture.services);
        assert!(!missing.pin_available());
        assert!(!available(&missing, TOOL_PIN));
        assert!(!available(&missing, TOOL_EDITOR));
        assert!(!available(&missing, "tool_qr"));
        assert_eq!(
            fixture
                .services
                .tool()
                .handle(ToolReq::Available {
                    tool: String::from(TOOL_PIN),
                })
                .expect("available"),
            ToolResp::Available { available: false }
        );

        let ports = Ports::default();
        let installed = editor(&fixture, &ports);
        assert!(installed.pin_available());
        assert!(available(&installed, TOOL_PIN));
        assert!(available(&installed, TOOL_EDITOR));
        assert!(!available(&installed, "tool_qr"));
        assert!(PinService::with_editor(installed).available());
    }

    #[test]
    fn a_file_pin_decodes_the_picked_image() {
        let fixture = fixture();
        let path = fixture.paths.captures_dir.join("picked.png");
        std::fs::write(&path, [b'b']).expect("file");
        let ports = Ports::default();
        *ports.dialog.path.lock().expect("path") = Some(path);
        let response = PinService::with_editor(editor(&fixture, &ports))
            .file()
            .expect("pin");
        assert_eq!(
            response,
            ToolResp::Pinned {
                source: PinSourceDto::File
            }
        );
        assert_eq!(*ports.pin.images.lock().expect("pins"), vec![(40, 80)]);
    }

    #[test]
    fn a_clipboard_pin_uses_an_image_or_the_first_file() {
        let fixture = fixture();
        let ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(5, 6)));
        let response = PinService::with_editor(editor(&fixture, &ports))
            .clipboard()
            .expect("clipboard");
        assert_eq!(
            response,
            ToolResp::Pinned {
                source: PinSourceDto::Clipboard
            }
        );
        assert_eq!(*ports.pin.images.lock().expect("pins"), vec![(5, 6)]);

        let first = fixture.paths.captures_dir.join("first.png");
        let second = fixture.paths.captures_dir.join("second.png");
        std::fs::write(&first, [b'c']).expect("first");
        std::fs::write(&second, [b'b']).expect("second");
        let ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Files(vec![first, second]));
        let response = PinService::with_editor(editor(&fixture, &ports))
            .clipboard()
            .expect("clipboard");
        assert_eq!(
            response,
            ToolResp::Pinned {
                source: PinSourceDto::Clipboard
            }
        );
        assert_eq!(*ports.pin.images.lock().expect("pins"), vec![(20, 10)]);
    }

    #[test]
    fn a_cancelled_dialog_pins_nothing() {
        let fixture = fixture();
        let ports = Ports::default();
        let response = PinService::with_editor(editor(&fixture, &ports))
            .file()
            .expect("pin");
        assert_eq!(response, ToolResp::PinCancelled);
        assert!(ports.pin.images.lock().expect("pins").is_empty());
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let fixture = fixture();
        let ports = Ports::default();
        *ports.dialog.path.lock().expect("path") =
            Some(fixture.paths.captures_dir.join("missing.png"));
        assert_eq!(
            PinService::with_editor(editor(&fixture, &ports))
                .file()
                .expect_err("missing"),
            ServiceError::NotFound
        );
    }

    #[test]
    fn non_images_and_empty_clipboards_are_rejected() {
        let fixture = fixture();
        let text = fixture.paths.captures_dir.join("notes.txt");
        std::fs::write(&text, b"text").expect("file");
        let ports = Ports::default();
        *ports.dialog.path.lock().expect("path") = Some(text.clone());
        assert!(matches!(
            PinService::with_editor(editor(&fixture, &ports)).file(),
            Err(ServiceError::Invalid(_))
        ));

        let ports = Ports::default();
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Files(vec![text]));
        assert!(matches!(
            PinService::with_editor(editor(&fixture, &ports)).clipboard(),
            Err(ServiceError::Invalid(_))
        ));

        let ports = Ports::default();
        assert!(matches!(
            PinService::with_editor(editor(&fixture, &ports)).clipboard(),
            Err(ServiceError::Invalid(_))
        ));
        assert!(ports.pin.images.lock().expect("pins").is_empty());
    }

    #[test]
    fn pin_handles_are_retained_across_requests() {
        let fixture = fixture();
        let path = fixture.paths.captures_dir.join("kept.png");
        std::fs::write(&path, [b'a']).expect("file");
        let ports = Ports::default();
        *ports.dialog.path.lock().expect("path") = Some(path);
        *ports.source.content.lock().expect("content") =
            Some(EditorClipboardContent::Image(image(2, 3)));
        {
            let service = PinService::with_editor(editor(&fixture, &ports));
            service.file().expect("file");
            service.clipboard().expect("clipboard");
        }
        assert_eq!(
            *ports.pin.images.lock().expect("pins"),
            vec![(100, 50), (2, 3)]
        );
    }

    #[test]
    fn pin_requests_need_installed_ports() {
        let fixture = fixture();
        assert_eq!(
            fixture
                .services
                .tool()
                .handle(ToolReq::PinFile)
                .expect_err("ports"),
            ServiceError::NotYet
        );
        assert_eq!(
            fixture
                .services
                .tool()
                .handle(ToolReq::PinClipboard)
                .expect_err("ports"),
            ServiceError::NotYet
        );
    }

    #[test]
    fn pin_requests_and_responses_use_the_ipc_kinds() {
        assert_eq!(
            serde_json::to_value(ToolReq::PinFile).expect("request"),
            serde_json::json!({ "kind": "pinFile" })
        );
        assert_eq!(
            serde_json::to_value(ToolReq::PinClipboard).expect("request"),
            serde_json::json!({ "kind": "pinClipboard" })
        );
        assert_eq!(
            serde_json::to_value(ToolResp::Pinned {
                source: PinSourceDto::Clipboard
            })
            .expect("response"),
            serde_json::json!({ "kind": "pinned", "data": { "source": "clipboard" } })
        );
        assert_eq!(
            serde_json::to_value(ToolResp::PinCancelled).expect("response"),
            serde_json::json!({ "kind": "pinCancelled" })
        );
    }
}
