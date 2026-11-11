use snapline_core::image::Image;
use snapline_core::ports::{
    Clipboard as ClipboardPort, ClipboardError as PortClipboardError, CodecError,
    ImageCodec as ImageCodecPort, Notice, Notifier, RichNotice, SecretError as PortSecretError,
    SecretStore as SecretStorePort, Sound, SoundId as PortSoundId,
};
use snapline_core::secret::{Secret, SecretKey};

use crate::clipboard::{Clipboard, ClipboardError};
use crate::credentials::{CredentialError, CredentialStore};
use crate::notify::{self, Notice as PlatformNotice};
use crate::sound::{self, SoundId};
use crate::wic::{WicCodec, WicError};

impl ClipboardPort for Clipboard {
    fn set_image(&self, image: &Image) -> Result<(), PortClipboardError> {
        Clipboard::set_image(self, image).map_err(map_clipboard_error)
    }

    fn set_text(&self, text: &str) -> Result<(), PortClipboardError> {
        Clipboard::set_text(self, text).map_err(map_clipboard_error)
    }

    fn get_image(&self) -> Result<Option<Image>, PortClipboardError> {
        Clipboard::get_image(self).map_err(map_clipboard_error)
    }
}

fn map_clipboard_error(error: ClipboardError) -> PortClipboardError {
    match error {
        ClipboardError::Busy => PortClipboardError::Busy,
        ClipboardError::Read(_) | ClipboardError::Unsupported | ClipboardError::Malformed => {
            PortClipboardError::Read
        }
        ClipboardError::Empty(_)
        | ClipboardError::Format(_)
        | ClipboardError::Allocate(_)
        | ClipboardError::Store(_) => PortClipboardError::Write,
    }
}

impl ImageCodecPort for WicCodec {
    fn encode(
        &self,
        image: &Image,
        format: snapline_core::image::ImageFormat,
        quality: u8,
    ) -> Result<Vec<u8>, CodecError> {
        WicCodec::encode(self, image, format, quality).map_err(map_codec_error)
    }

    fn decode(&self, bytes: &[u8]) -> Result<Image, CodecError> {
        WicCodec::decode(self, bytes).map_err(map_codec_error)
    }
}

fn map_codec_error(error: WicError) -> CodecError {
    match error {
        WicError::WebpUnavailable => CodecError::Unsupported,
        WicError::Decode => CodecError::Decode(error.to_string()),
        WicError::Encode => CodecError::Encode(error.to_string()),
        WicError::Com | WicError::Factory | WicError::Dimensions => {
            CodecError::Encode(error.to_string())
        }
    }
}

impl SecretStorePort for CredentialStore {
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, PortSecretError> {
        CredentialStore::get(self, key).map_err(map_secret_error)
    }

    fn put(&self, key: &SecretKey, value: &Secret) -> Result<(), PortSecretError> {
        CredentialStore::put(self, key, value).map_err(map_secret_error)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), PortSecretError> {
        CredentialStore::delete(self, key).map_err(map_secret_error)
    }
}

fn map_secret_error(error: CredentialError) -> PortSecretError {
    match error {
        CredentialError::ValueTooLarge => PortSecretError::TooLarge,
        CredentialError::Read(_) => PortSecretError::Read,
        CredentialError::Write(_) => PortSecretError::Write,
        CredentialError::Delete(_) => PortSecretError::Delete,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ToastNotifier;

impl Notifier for ToastNotifier {
    fn toast(&self, notice: Notice) {
        let platform = PlatformNotice {
            title: notice.title,
            body: notice.body,
            capture_id: notice.capture_id.and_then(|id| u64::try_from(id).ok()),
        };
        if let Err(error) = notify::show(&platform) {
            tracing::warn!(%error, "could not show a notification");
        }
    }

    fn toast_open(&self, notice: Notice, target: String) {
        let platform = PlatformNotice {
            title: notice.title,
            body: notice.body,
            capture_id: None,
        };
        if let Err(error) = notify::show_with_target(&platform, &target) {
            tracing::warn!(%error, "could not show a notification");
        }
    }

    fn toast_rich(&self, notice: RichNotice) {
        let platform = notify::RichNotice {
            title: notice.title,
            lines: notice.lines,
            image: notice.image,
            open: notice.open,
            tag: notice.tag,
            buttons: notice
                .buttons
                .into_iter()
                .map(|button| notify::RichButton {
                    label: button.label,
                    target: button.target,
                })
                .collect(),
        };
        if let Err(error) = notify::show_rich(&platform) {
            tracing::warn!(%error, "could not show a notification");
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SoundPlayer;

impl Sound for SoundPlayer {
    fn play(&self, sound: PortSoundId) {
        let platform = match sound {
            PortSoundId::Shutter => SoundId::Shutter,
            PortSoundId::Done => SoundId::Done,
        };
        sound::play(platform);
    }
}
