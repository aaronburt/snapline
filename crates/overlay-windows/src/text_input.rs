use snapline_core::annotate::Colour;
use snapline_core::geometry::{Point, Rect};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, SetBkColor, SetTextColor, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, FF_DONTCARE, FW_BOLD, HBRUSH, HDC, HFONT, HGDIOBJ,
    OUT_DEFAULT_PRECIS, VARIABLE_PITCH,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VIRTUAL_KEY, VK_ESCAPE, VK_RETURN, VK_SHIFT,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowTextLengthW, GetWindowTextW, PostMessageW,
    SendMessageW, SetWindowTextW, ES_AUTOHSCROLL, ES_LEFT, ES_MULTILINE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFONT, WM_SYSKEYDOWN, WS_CHILD,
    WS_VISIBLE,
};

pub(crate) const COMMIT_MESSAGE: u32 = WM_APP + 1;
pub(crate) const CANCEL_MESSAGE: u32 = WM_APP + 2;
pub(crate) const CHANGE_MESSAGE: u32 = WM_APP + 3;

const SUBCLASS_ID: usize = 0x534E_4C31;

struct Subclass {
    parent: HWND,
}

pub(crate) struct TextInput {
    hwnd: HWND,
    origin: Point,
    area: Rect,
    font: HFONT,
    brush: HBRUSH,
    background: COLORREF,
    foreground: COLORREF,
    _subclass: Box<Subclass>,
}

impl TextInput {
    pub(crate) fn create(
        parent: HWND,
        area: Rect,
        origin: Point,
        colour: Colour,
        scale: f32,
    ) -> Option<Self> {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let style = WINDOW_STYLE(
            WS_CHILD.0
                | WS_VISIBLE.0
                | ES_LEFT as u32
                | ES_MULTILINE as u32
                | ES_AUTOHSCROLL as u32,
        );
        // SAFETY: the class name is a static null terminated string and the parent handle is live.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR::null(),
                style,
                area.x.get(),
                area.y.get(),
                area.width.0.max(1),
                area.height.0.max(1),
                Some(parent),
                None,
                None,
                None,
            )
        }
        .ok()?;
        let (background, foreground) = palette(colour);
        // SAFETY: creates a bold GDI font owned by the returned struct.
        let font = unsafe {
            CreateFontW(
                -((16.0 * scale).round() as i32),
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                VARIABLE_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
                w!("Segoe UI"),
            )
        };
        // SAFETY: creates a solid background brush owned by the returned struct.
        let brush = unsafe { CreateSolidBrush(background) };
        let subclass = Box::new(Subclass { parent });
        let data = (&*subclass as *const Subclass) as usize;
        // SAFETY: installs a subclass whose reference data outlives the edit window.
        let installed = unsafe { SetWindowSubclass(hwnd, Some(edit_subclass), SUBCLASS_ID, data) };
        if !installed.as_bool() {
            // SAFETY: releases the window and GDI objects created above.
            unsafe {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(HGDIOBJ(font.0));
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
            return None;
        }
        // SAFETY: applies the owned font to the live edit control.
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let input = Self {
            hwnd,
            origin,
            area,
            font,
            brush,
            background,
            foreground,
            _subclass: subclass,
        };
        input.set_text("");
        input.focus();
        Some(input)
    }

    pub(crate) fn origin(&self) -> Point {
        self.origin
    }

    pub(crate) fn area(&self) -> Rect {
        self.area
    }

    pub(crate) fn contains(&self, point: Point) -> bool {
        self.area().contains(point)
    }

    pub(crate) fn paint(&self, hdc: HDC) -> HBRUSH {
        // SAFETY: the device context belongs to the edit control being painted.
        unsafe {
            let _ = SetBkColor(hdc, self.background);
            let _ = SetTextColor(hdc, self.foreground);
        }
        self.brush
    }

    pub(crate) fn set_text(&self, text: &str) {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the buffer is null terminated and lives for the duration of the call.
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR::from_raw(wide.as_ptr()));
        }
    }

    pub(crate) fn text(&self) -> String {
        // SAFETY: reads the current text length of the live edit control.
        let length = unsafe { GetWindowTextLengthW(self.hwnd) }.max(0) as usize;
        let mut buffer = vec![0_u16; length + 1];
        // SAFETY: the buffer has room for the text plus a null terminator.
        let copied = unsafe { GetWindowTextW(self.hwnd, &mut buffer) }.max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied.min(length)])
    }

    pub(crate) fn focus(&self) {
        // SAFETY: the edit control is a live child window.
        unsafe {
            let _ = SetFocus(Some(self.hwnd));
        }
    }
}

impl Drop for TextInput {
    fn drop(&mut self) {
        // SAFETY: removes the subclass, destroys the child and releases the owned GDI objects once.
        unsafe {
            let _ = RemoveWindowSubclass(self.hwnd, Some(edit_subclass), SUBCLASS_ID);
            let _ = DestroyWindow(self.hwnd);
            let _ = DeleteObject(HGDIOBJ(self.font.0));
            let _ = DeleteObject(HGDIOBJ(self.brush.0));
        }
    }
}

unsafe extern "system" fn edit_subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if data != 0 && (message == WM_KEYDOWN || message == WM_SYSKEYDOWN) {
        // SAFETY: the reference data points to the Box<Subclass> owned by the live TextInput.
        let state = unsafe { &*(data as *const Subclass) };
        let key = VIRTUAL_KEY(wparam.0 as u16);
        if key == VK_ESCAPE {
            // SAFETY: posts to the live parent overlay window.
            let _ =
                unsafe { PostMessageW(Some(state.parent), CANCEL_MESSAGE, WPARAM(0), LPARAM(0)) };
            return LRESULT(0);
        }
        if key == VK_RETURN {
            // SAFETY: reads the calling thread's key state.
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
            if !shift {
                // SAFETY: posts to the live parent overlay window.
                let _ = unsafe {
                    PostMessageW(Some(state.parent), COMMIT_MESSAGE, WPARAM(0), LPARAM(0))
                };
                return LRESULT(0);
            }
        }
    }
    if data != 0 && message == WM_KILLFOCUS {
        // SAFETY: the reference data points to the Box<Subclass> owned by the live TextInput.
        let state = unsafe { &*(data as *const Subclass) };
        // SAFETY: posts to the live parent overlay window.
        let _ = unsafe { PostMessageW(Some(state.parent), COMMIT_MESSAGE, WPARAM(0), LPARAM(0)) };
    }
    // SAFETY: forwards every other message to the edit control's default procedure.
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if data != 0 && matches!(message, 0x0102 | 0x0302 | 0x0300 | 0x0303 | 0x0304) {
        let state = unsafe { &*(data as *const Subclass) };
        let _ = unsafe { PostMessageW(Some(state.parent), CHANGE_MESSAGE, WPARAM(0), LPARAM(0)) };
    }
    result
}

fn palette(colour: Colour) -> (COLORREF, COLORREF) {
    let rgba = colour.rgba();
    if colour.is_white() {
        (COLORREF(0x0014_1414), COLORREF(0x00FF_FFFF))
    } else {
        (
            COLORREF(0x00FF_FFFF),
            COLORREF((u32::from(rgba.b) << 16) | (u32::from(rgba.g) << 8) | u32::from(rgba.r)),
        )
    }
}

#[cfg(test)]
mod tests {
    use snapline_core::geometry::Px;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn native_edit_round_trips_text_and_area() {
        // SAFETY: the desktop window is a valid parent for a hidden child in a desktop test.
        let parent = unsafe { GetDesktopWindow() };
        let area = Rect::new(Px(100), Px(100), Px(200), Px(28));
        let Some(input) =
            TextInput::create(parent, area, Point::new(Px(10), Px(10)), Colour::Red, 1.0)
        else {
            return;
        };
        input.set_text("Label");
        assert_eq!(input.text(), "Label");
        assert_eq!(input.origin(), Point::new(Px(10), Px(10)));
        assert_eq!(input.area(), area);
        assert!(input.contains(Point::new(Px(120), Px(110))));
    }
}
