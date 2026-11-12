use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use snapline_core::ports::{NoticeButtonTarget, RichNotice};
use thiserror::Error;
use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, RPC_E_CHANGED_MODE};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
};

pub const APP_USER_MODEL_ID: &str = "io.github.aaronburt.snapline";
pub const APP_USER_MODEL_KEY: &str =
    "Software\\Classes\\AppUserModelId\\io.github.aaronburt.snapline";
pub const DISPLAY_NAME_VALUE: &str = "DisplayName";
pub const ICON_URI_VALUE: &str = "IconUri";
pub const DISPLAY_NAME: &str = "Snapline";
pub const ACTIVATION_SCHEME: &str = "snapline";
pub const ACTIVATION_CAPTURE_PREFIX: &str = "snapline://capture/";
const ACTIVATION_COPY_IMAGE_PREFIX: &str = "copy-image:";

const MAX_LIVE_TOASTS: usize = 16;
const TOAST_GROUP: &str = "snapline-capture";

thread_local! {
    static LIVE_TOASTS: RefCell<Vec<ToastNotification>> = RefCell::new(Vec::new());
}

static ACTIVATION_HANDLER: OnceLock<Mutex<Option<ActivationCallback>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastActivation {
    OpenCapture(u64),
    CopyImage(i64),
}

pub type ActivationCallback = Arc<dyn Fn(ToastActivation) + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub capture_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotifyError {
    #[error("the icon path does not exist: {0:?}")]
    IconNotFound(PathBuf),
    #[error("the icon path is not a PNG file: {0:?}")]
    IconNotPng(PathBuf),
    #[error("the icon path could not be made absolute: {0:?}")]
    InvalidIconPath(PathBuf),
    #[error("the notification has no title and no body")]
    EmptyNotice,
    #[error("the Windows registry returned error {0}")]
    Registry(u32),
    #[error("the Windows notification API failed: {0}")]
    Platform(String),
}

impl Notice {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            capture_id: None,
        }
    }

    pub fn for_capture(title: impl Into<String>, body: impl Into<String>, capture_id: u64) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            capture_id: Some(capture_id),
        }
    }
}

struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: self.0 is an open registry key owned by this guard and is closed exactly once here.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

pub fn register_app_id(icon_uri: impl AsRef<Path>) -> Result<(), NotifyError> {
    register_app_id_at(APP_USER_MODEL_KEY, icon_uri.as_ref())
}

pub fn remove_app_id() -> Result<(), NotifyError> {
    remove_app_id_at(APP_USER_MODEL_KEY)
}

pub fn capture_launch(capture_id: u64) -> String {
    format!("{ACTIVATION_CAPTURE_PREFIX}{capture_id}")
}

pub fn parse_capture_id(arguments: &str) -> Option<u64> {
    arguments
        .strip_prefix(ACTIVATION_CAPTURE_PREFIX)
        .and_then(|id| id.parse().ok())
}

fn parse_activation(arguments: &str) -> Option<ToastActivation> {
    if let Some(id) = arguments.strip_prefix(ACTIVATION_COPY_IMAGE_PREFIX) {
        return id
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .map(ToastActivation::CopyImage);
    }
    parse_capture_id(arguments).map(ToastActivation::OpenCapture)
}

pub fn set_activation_handler<F>(handler: F)
where
    F: Fn(ToastActivation) + Send + Sync + 'static,
{
    let slot = ACTIVATION_HANDLER.get_or_init(|| Mutex::new(None));
    let mut guard = match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(Arc::new(handler));
}

pub fn clear_activation_handler() {
    let slot = ACTIVATION_HANDLER.get_or_init(|| Mutex::new(None));
    let mut guard = match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = None;
}

pub fn toast(title: &str, body: &str) -> Result<(), NotifyError> {
    show(&Notice::new(title, body))
}

pub fn show(notice: &Notice) -> Result<(), NotifyError> {
    if notice.title.is_empty() && notice.body.is_empty() {
        return Err(NotifyError::EmptyNotice);
    }
    show_xml(toast_xml(notice))
}

pub fn show_with_target(notice: &Notice, target: &str) -> Result<(), NotifyError> {
    if notice.title.is_empty() && notice.body.is_empty() {
        return Err(NotifyError::EmptyNotice);
    }
    show_xml(toast_xml_with_target(notice, target))
}

const MAX_BUTTONS: usize = 5;

pub fn show_rich(notice: &RichNotice) -> Result<(), NotifyError> {
    if notice.title.is_empty() && notice.lines.iter().all(String::is_empty) {
        return Err(NotifyError::EmptyNotice);
    }
    show_xml_tagged(rich_xml(notice), notice.tag.as_deref())
}

fn rich_xml(notice: &RichNotice) -> String {
    let mut texts = String::new();
    if !notice.title.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape_xml(&notice.title)));
    }
    for line in notice.lines.iter().filter(|line| !line.is_empty()) {
        texts.push_str(&format!("<text>{}</text>", escape_xml(line)));
    }
    if let Some(image) = &notice.image {
        texts.push_str(&format!(
            "<image placement=\"inline\" src=\"{}\"/>",
            escape_xml(&folder_target(image))
        ));
    }
    let launch = notice
        .open
        .as_deref()
        .filter(|target| is_launchable_target(target))
        .map(|target| {
            format!(
                " activationType=\"protocol\" launch=\"{}\"",
                escape_xml(target)
            )
        })
        .unwrap_or_default();
    let actions: String = notice
        .buttons
        .iter()
        .filter(|button| !button.label.is_empty())
        .filter_map(|button| {
            let (activation, target) = match &button.target {
                NoticeButtonTarget::Open(url) if is_launchable_target(url) => {
                    ("protocol", Cow::Borrowed(url.as_str()))
                }
                NoticeButtonTarget::CopyImage(id) if *id > 0 => (
                    "foreground",
                    Cow::Owned(format!("{ACTIVATION_COPY_IMAGE_PREFIX}{id}")),
                ),
                _ => return None,
            };
            Some(format!(
                "<action content=\"{}\" activationType=\"{activation}\" arguments=\"{}\"/>",
                escape_xml(&button.label),
                escape_xml(&target)
            ))
        })
        .take(MAX_BUTTONS)
        .collect();
    let actions = if actions.is_empty() {
        String::new()
    } else {
        format!("<actions>{actions}</actions>")
    };
    format!(
        "<toast{launch}><visual><binding template=\"ToastGeneric\">{texts}</binding></visual>{actions}</toast>"
    )
}

pub fn folder_target(folder: &Path) -> String {
    let mut url = String::from("file:///");
    for character in folder.to_string_lossy().chars() {
        match character {
            '\\' => url.push('/'),
            ' ' => url.push_str("%20"),
            '#' => url.push_str("%23"),
            '%' => url.push_str("%25"),
            '?' => url.push_str("%3F"),
            other => url.push(other),
        }
    }
    url
}

fn is_launchable_target(target: &str) -> bool {
    ["https://", "http://", "file:///"]
        .iter()
        .any(|scheme| target.starts_with(scheme))
}

fn toast_xml_with_target(notice: &Notice, target: &str) -> String {
    if !is_launchable_target(target) {
        return toast_xml(notice);
    }
    let mut texts = String::new();
    if !notice.title.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape_xml(&notice.title)));
    }
    if !notice.body.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape_xml(&notice.body)));
    }
    format!(
        "<toast activationType=\"protocol\" launch=\"{}\"><visual><binding template=\"ToastGeneric\">{texts}</binding></visual></toast>",
        escape_xml(target)
    )
}

fn usable_tag(tag: Option<&str>) -> Option<&str> {
    tag.filter(|tag| !tag.is_empty() && tag.len() <= 64)
}

fn show_xml(xml: String) -> Result<(), NotifyError> {
    show_xml_tagged(xml, None)
}

fn show_xml_tagged(xml: String, tag: Option<&str>) -> Result<(), NotifyError> {
    ensure_winrt()?;
    let content = XmlDocument::new().map_err(platform_error)?;
    content
        .LoadXml(&HSTRING::from(xml))
        .map_err(platform_error)?;
    let toast = ToastNotification::CreateToastNotification(&content).map_err(platform_error)?;
    if let Some(tag) = usable_tag(tag) {
        toast.SetTag(&HSTRING::from(tag)).map_err(platform_error)?;
        toast
            .SetGroup(&HSTRING::from(TOAST_GROUP))
            .map_err(platform_error)?;
    }
    let handler = TypedEventHandler::<ToastNotification, windows::core::IInspectable>::new(
        |_sender, args| {
            if let Some(args) = args.as_ref() {
                if let Ok(args) = args.cast::<ToastActivatedEventArgs>() {
                    if let Ok(arguments) = args.Arguments() {
                        dispatch_activation(&arguments.to_string());
                    }
                }
            }
            Ok(())
        },
    );
    toast.Activated(&handler).map_err(platform_error)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_USER_MODEL_ID))
            .map_err(platform_error)?;
    notifier.Show(&toast).map_err(platform_error)?;
    keep_alive(toast);
    Ok(())
}

fn keep_alive(toast: ToastNotification) {
    LIVE_TOASTS.with(|live| {
        let mut live = live.borrow_mut();
        if live.len() >= MAX_LIVE_TOASTS {
            live.remove(0);
        }
        live.push(toast);
    });
}

fn dispatch_activation(arguments: &str) {
    let Some(activation) = parse_activation(arguments) else {
        return;
    };
    let callback = {
        let slot = ACTIVATION_HANDLER.get_or_init(|| Mutex::new(None));
        let guard = match slot.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clone()
    };
    if let Some(callback) = callback {
        callback(activation);
    }
}

fn ensure_winrt() -> Result<(), NotifyError> {
    // SAFETY: RoInitialize takes no pointers and only initialises a WinRT apartment on this thread.
    match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(()),
        Err(error) => Err(platform_error(error)),
    }
}

fn platform_error(error: windows::core::Error) -> NotifyError {
    NotifyError::Platform(error.message())
}

fn toast_xml(notice: &Notice) -> String {
    let launch = notice
        .capture_id
        .map(|capture_id| format!(" launch=\"{}\"", escape_xml(&capture_launch(capture_id))))
        .unwrap_or_default();
    let mut texts = String::new();
    if !notice.title.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape_xml(&notice.title)));
    }
    if !notice.body.is_empty() {
        texts.push_str(&format!("<text>{}</text>", escape_xml(&notice.body)));
    }
    format!(
        "<toast activationType=\"foreground\"{launch}><visual><binding template=\"ToastGeneric\">{texts}</binding></visual></toast>"
    )
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

pub fn register_app_name() -> Result<(), NotifyError> {
    register_with_icon(APP_USER_MODEL_KEY, None)
}

fn register_app_id_at(subkey: &str, icon: &Path) -> Result<(), NotifyError> {
    register_with_icon(subkey, validated_icon(icon).ok().as_deref())
}

fn register_with_icon(subkey: &str, icon: Option<&Path>) -> Result<(), NotifyError> {
    let key = open_key(subkey)?;
    set_value(key.0, DISPLAY_NAME_VALUE, DISPLAY_NAME)?;
    if let Some(icon) = icon {
        set_value(key.0, ICON_URI_VALUE, &icon.to_string_lossy())?;
    }
    Ok(())
}

fn remove_app_id_at(subkey: &str) -> Result<(), NotifyError> {
    let subkey = wide(subkey);
    for value in [DISPLAY_NAME_VALUE, ICON_URI_VALUE] {
        let value = wide(value);
        // SAFETY: both subkey and value are live null-terminated UTF-16 strings valid for the call.
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
            )
        };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return Err(NotifyError::Registry(status.0));
        }
    }
    Ok(())
}

fn validated_icon(icon: &Path) -> Result<PathBuf, NotifyError> {
    if !icon.is_file() {
        return Err(NotifyError::IconNotFound(icon.to_path_buf()));
    }
    let is_png = icon
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(NotifyError::IconNotPng(icon.to_path_buf()));
    }
    std::path::absolute(icon).map_err(|_| NotifyError::InvalidIconPath(icon.to_path_buf()))
}

fn open_key(subkey: &str) -> Result<RegKey, NotifyError> {
    let subkey = wide(subkey);
    let mut key = HKEY::default();
    // SAFETY: subkey is a live null-terminated UTF-16 string and key points to writable HKEY storage.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(RegKey(key))
    } else {
        Err(NotifyError::Registry(status.0))
    }
}

fn set_value(key: HKEY, name: &str, value: &str) -> Result<(), NotifyError> {
    let name = wide(name);
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = Vec::with_capacity(data.len() * 2);
    for unit in &data {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    // SAFETY: name is a live null-terminated UTF-16 string and bytes is a live REG_SZ data buffer.
    let status = unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&bytes)) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(NotifyError::Registry(status.0))
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapline_core::ports::NoticeButton as RichButton;
    use windows::Win32::System::Registry::{RegDeleteKeyW, RegGetValueW, RRF_RT_REG_SZ};

    const TEST_KEY: &str = "SnapLineTest:io.github.aaronburt.snapline";
    const SENTINEL_VALUE: &str = "Sentinel";

    struct TestKey(&'static str);

    impl Drop for TestKey {
        fn drop(&mut self) {
            delete_value(self.0, DISPLAY_NAME_VALUE);
            delete_value(self.0, ICON_URI_VALUE);
            delete_value(self.0, SENTINEL_VALUE);
            let subkey = wide(self.0);
            // SAFETY: subkey is a live null-terminated UTF-16 string naming the test-only key.
            let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr())) };
        }
    }

    #[test]
    fn a_url_target_becomes_a_protocol_launch_attribute() {
        let notice = Notice::new("Upload complete", "https://example.test/a.png?x=1&y=2");
        let xml = toast_xml_with_target(&notice, "https://example.test/a.png?x=1&y=2");
        assert!(xml.starts_with(
            "<toast activationType=\"protocol\" launch=\"https://example.test/a.png?x=1&amp;y=2\">"
        ));
        assert!(xml.contains("<text>Upload complete</text>"));
    }

    #[test]
    fn a_folder_target_becomes_a_file_url_launch_attribute() {
        let target = folder_target(Path::new("C:\\Users\\Aaron\\My Pictures\\Snapline"));
        assert_eq!(target, "file:///C:/Users/Aaron/My%20Pictures/Snapline");
        let xml = toast_xml_with_target(&Notice::new("Upload complete", "C:\\x"), &target);
        assert!(xml.contains(
            "activationType=\"protocol\" launch=\"file:///C:/Users/Aaron/My%20Pictures/Snapline\""
        ));
    }

    fn sample_rich() -> RichNotice {
        RichNotice {
            title: "Screenshot saved".to_string(),
            lines: vec!["a & b.png".to_string(), "1920x1080 from <App>".to_string()],
            image: Some(PathBuf::from("C:\\Thumbs\\my thumb.webp")),
            open: Some("file:///C:/Shots/a.png".to_string()),
            tag: Some("capture-7".to_string()),
            buttons: vec![
                RichButton::open("Open image", "file:///C:/Shots/a.png".to_string()),
                RichButton::open("Open folder", "file:///C:/Shots".to_string()),
            ],
        }
    }

    #[test]
    fn a_rich_toast_carries_text_image_click_target_and_buttons() {
        let xml = rich_xml(&sample_rich());
        assert!(xml
            .starts_with("<toast activationType=\"protocol\" launch=\"file:///C:/Shots/a.png\">"));
        assert!(xml.contains("<text>Screenshot saved</text>"));
        assert!(xml.contains("<text>a &amp; b.png</text>"));
        assert!(xml.contains("<text>1920x1080 from &lt;App&gt;</text>"));
        assert!(
            xml.contains("<image placement=\"inline\" src=\"file:///C:/Thumbs/my%20thumb.webp\"/>")
        );
        assert!(xml.contains(
            "<action content=\"Open image\" activationType=\"protocol\" arguments=\"file:///C:/Shots/a.png\"/>"
        ));
        assert!(xml.contains(
            "<action content=\"Open folder\" activationType=\"protocol\" arguments=\"file:///C:/Shots\"/>"
        ));
    }

    #[test]
    fn a_rich_toast_drops_unsafe_targets_and_buttons() {
        let mut notice = sample_rich();
        notice.open = Some("javascript:alert(1)".to_string());
        notice.buttons = vec![
            RichButton::open("Run", "ms-settings:privacy".to_string()),
            RichButton::open(
                "Open link",
                "https://example.test/a.png?x=1&y=2".to_string(),
            ),
            RichButton::open("Forged copy", "copy-image:7".to_string()),
        ];
        let xml = rich_xml(&notice);
        assert!(!xml.contains("javascript"));
        assert!(!xml.contains("ms-settings"));
        assert!(!xml.contains("launch="));
        assert!(!xml.contains("copy-image:7"));
        assert!(xml.contains("arguments=\"https://example.test/a.png?x=1&amp;y=2\""));
    }

    #[test]
    fn a_rich_toast_without_image_or_buttons_is_plain() {
        let xml = rich_xml(&RichNotice {
            title: "Hi".to_string(),
            ..RichNotice::default()
        });
        assert!(!xml.contains("<image"));
        assert!(!xml.contains("<actions>"));
        assert!(xml.contains("<text>Hi</text>"));
    }

    #[test]
    fn a_toast_tag_is_used_only_when_set_and_valid() {
        assert_eq!(usable_tag(Some("capture-7")), Some("capture-7"));
        assert_eq!(usable_tag(Some("")), None);
        assert_eq!(usable_tag(None), None);
        assert_eq!(usable_tag(Some(&"x".repeat(65))), None);
        assert_eq!(TOAST_GROUP, "snapline-capture");
    }

    #[test]
    fn a_rich_toast_caps_the_button_count() {
        let mut notice = sample_rich();
        notice.buttons = (0..9)
            .map(|index| RichButton::open(format!("b{index}"), "https://example.test/".to_string()))
            .collect();
        assert_eq!(rich_xml(&notice).matches("<action ").count(), MAX_BUTTONS);
    }

    #[test]
    fn a_copy_button_emits_an_app_activation_for_the_saved_image() {
        let mut notice = sample_rich();
        notice.buttons.push(RichButton {
            label: "Copy image".to_owned(),
            target: NoticeButtonTarget::CopyImage(7),
        });
        let xml = rich_xml(&notice);
        assert_eq!(xml.matches("<action ").count(), 3);
        assert!(xml.contains(
            "<action content=\"Copy image\" activationType=\"foreground\" arguments=\"copy-image:7\"/>"
        ));
        assert_eq!(
            parse_activation("copy-image:7"),
            Some(ToastActivation::CopyImage(7))
        );
        assert_eq!(parse_activation("copy-image:0"), None);
        assert_eq!(parse_activation("copy-image:7/path"), None);
        assert_eq!(
            parse_activation("snapline://capture/7"),
            Some(ToastActivation::OpenCapture(7))
        );
    }

    #[test]
    fn an_empty_rich_notice_is_rejected() {
        assert_eq!(
            show_rich(&RichNotice::default()),
            Err(NotifyError::EmptyNotice)
        );
    }

    #[test]
    fn an_unsafe_target_scheme_is_not_launched() {
        let notice = Notice::new("Upload complete", "x");
        let xml = toast_xml_with_target(&notice, "javascript:alert(1)");
        assert!(!xml.contains("protocol"));
        assert!(!xml.contains("javascript"));
    }

    #[test]
    fn constants_identify_the_snapline_app_id() {
        assert_eq!(APP_USER_MODEL_ID, "io.github.aaronburt.snapline");
        assert_eq!(
            APP_USER_MODEL_KEY,
            "Software\\Classes\\AppUserModelId\\io.github.aaronburt.snapline"
        );
        assert_eq!(DISPLAY_NAME, "Snapline");
        assert_eq!(DISPLAY_NAME_VALUE, "DisplayName");
        assert_eq!(ICON_URI_VALUE, "IconUri");
        assert!(APP_USER_MODEL_KEY.starts_with("Software\\Classes\\AppUserModelId\\"));
        assert!(APP_USER_MODEL_KEY.ends_with(APP_USER_MODEL_ID));
    }

    #[test]
    fn capture_launch_round_trips() {
        assert_eq!(capture_launch(42), "snapline://capture/42");
        assert_eq!(parse_capture_id("snapline://capture/42"), Some(42));
        assert_eq!(parse_capture_id("snapline://capture/0"), Some(0));
        assert_eq!(parse_capture_id("snapline://capture/twelve"), None);
        assert_eq!(parse_capture_id("https://example.test/capture/42"), None);
        assert_eq!(parse_capture_id(""), None);
    }

    #[test]
    fn capture_toast_uses_a_foreground_activation_route() {
        let xml = toast_xml(&Notice::for_capture("Saved", "shot.png", 7));
        assert!(xml.contains("activationType=\"foreground\""));
        assert!(xml.contains("launch=\"snapline://capture/7\""));
        assert!(xml.contains("<text>Saved</text>"));
        assert!(xml.contains("<text>shot.png</text>"));
        assert!(xml.contains("<binding template=\"ToastGeneric\">"));
    }

    #[test]
    fn plain_toast_has_no_launch_argument() {
        let xml = toast_xml(&Notice::new("Hello", "World"));
        assert!(!xml.contains("launch="));
        assert!(xml.contains("activationType=\"foreground\""));
    }

    #[test]
    fn toast_text_is_xml_escaped() {
        let xml = toast_xml(&Notice::new("A & B", "<done> \"now\" 'yes'"));
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("&lt;done&gt; &quot;now&quot; &apos;yes&apos;"));
        assert!(!xml.contains("<done>"));
    }

    #[test]
    fn empty_notice_is_rejected_before_registration_or_winrt() {
        assert_eq!(show(&Notice::new("", "")), Err(NotifyError::EmptyNotice));
    }

    #[test]
    #[ignore = "desktop"]
    fn missing_icon_registers_the_name_only() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = TestKey(TEST_KEY);
        let directory = tempfile::tempdir()?;
        let icon = directory.path().join("missing.png");
        register_app_id_at(TEST_KEY, &icon)?;
        assert_eq!(
            read_string(TEST_KEY, DISPLAY_NAME_VALUE).as_deref(),
            Some(DISPLAY_NAME)
        );
        assert_eq!(read_string(TEST_KEY, ICON_URI_VALUE), None);
        Ok(())
    }

    #[test]
    #[ignore = "desktop"]
    fn non_png_icon_registers_the_name_only() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = TestKey(TEST_KEY);
        let directory = tempfile::tempdir()?;
        let icon = directory.path().join("icon.jpg");
        std::fs::write(&icon, [0_u8; 8])?;
        register_app_id_at(TEST_KEY, &icon)?;
        assert_eq!(
            read_string(TEST_KEY, DISPLAY_NAME_VALUE).as_deref(),
            Some(DISPLAY_NAME)
        );
        assert_eq!(read_string(TEST_KEY, ICON_URI_VALUE), None);
        Ok(())
    }

    #[test]
    fn invalid_icons_are_still_rejected_by_validation() {
        let Ok(directory) = tempfile::tempdir() else {
            return;
        };
        let missing = directory.path().join("missing.png");
        assert_eq!(
            validated_icon(&missing),
            Err(NotifyError::IconNotFound(missing.clone()))
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn app_id_registration_is_idempotent_and_removable() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = TestKey(TEST_KEY);
        let directory = tempfile::tempdir()?;
        let icon = directory.path().join("icon.png");
        std::fs::write(&icon, [0_u8; 8])?;
        set_value_at(TEST_KEY, SENTINEL_VALUE, "keep")?;
        register_app_id_at(TEST_KEY, &icon)?;
        register_app_id_at(TEST_KEY, &icon)?;
        let absolute = std::path::absolute(&icon)?.to_string_lossy().into_owned();
        assert_eq!(
            read_string(TEST_KEY, DISPLAY_NAME_VALUE).as_deref(),
            Some(DISPLAY_NAME)
        );
        assert_eq!(
            read_string(TEST_KEY, ICON_URI_VALUE).as_deref(),
            Some(absolute.as_str())
        );
        assert_eq!(
            read_string(TEST_KEY, SENTINEL_VALUE).as_deref(),
            Some("keep")
        );
        remove_app_id_at(TEST_KEY)?;
        assert_eq!(read_string(TEST_KEY, DISPLAY_NAME_VALUE), None);
        assert_eq!(read_string(TEST_KEY, ICON_URI_VALUE), None);
        assert_eq!(
            read_string(TEST_KEY, SENTINEL_VALUE).as_deref(),
            Some("keep")
        );
        Ok(())
    }

    fn set_value_at(subkey: &str, name: &str, value: &str) -> Result<(), NotifyError> {
        let key = open_key(subkey)?;
        set_value(key.0, name, value)
    }

    fn read_string(subkey: &str, name: &str) -> Option<String> {
        let subkey = wide(subkey);
        let name = wide(name);
        let mut buffer = [0_u16; 512];
        let mut bytes = u32::try_from(buffer.len() * std::mem::size_of::<u16>()).ok()?;
        // SAFETY: the buffer is a live writable array of 512 u16 units and bytes is its size in bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status.is_err() {
            return None;
        }
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..length]))
    }

    fn delete_value(subkey: &str, name: &str) {
        let subkey = wide(subkey);
        let name = wide(name);
        // SAFETY: both subkey and name are live null-terminated UTF-16 strings for the test key.
        let _ = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        };
    }
}
