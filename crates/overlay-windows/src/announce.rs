use snapline_core::geometry::Rect;
use windows::core::BSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{
    NotificationKind_Other, NotificationProcessing_ImportantMostRecent, UiaHostProviderFromHwnd,
    UiaRaiseNotificationEvent,
};

pub(crate) const ACTIVITY: &str = "snapline.overlay";

pub(crate) fn selection_text(selection: Rect) -> String {
    format!(
        "{} by {} pixels, X {}, Y {}",
        selection.width.get(),
        selection.height.get(),
        selection.x.get(),
        selection.y.get()
    )
}

pub(crate) fn notify(hwnd: HWND, text: &str) {
    if text.is_empty() {
        return;
    }
    // SAFETY: the window handle belongs to this thread and the returned provider owns its reference.
    let Ok(provider) = (unsafe { UiaHostProviderFromHwnd(hwnd) }) else {
        return;
    };
    let display = BSTR::from(text);
    let activity = BSTR::from(ACTIVITY);
    // SAFETY: the provider is live, both strings outlive the call and UI Automation may be absent.
    unsafe {
        let _ = UiaRaiseNotificationEvent(
            &provider,
            NotificationKind_Other,
            NotificationProcessing_ImportantMostRecent,
            &display,
            &activity,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_text_uses_physical_pixels_and_coordinates() {
        let selection = Rect::new(
            snapline_core::geometry::Px(280),
            snapline_core::geometry::Px(160),
            snapline_core::geometry::Px(660),
            snapline_core::geometry::Px(500),
        );
        assert_eq!(selection_text(selection), "660 by 500 pixels, X 280, Y 160");
    }

    #[test]
    fn selection_text_reports_a_single_pixel() {
        let selection = Rect::new(
            snapline_core::geometry::Px(0),
            snapline_core::geometry::Px(0),
            snapline_core::geometry::Px(1),
            snapline_core::geometry::Px(1),
        );
        assert_eq!(selection_text(selection), "1 by 1 pixels, X 0, Y 0");
    }
}
