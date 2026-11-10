use snapline_core::geometry::Rect;
use thiserror::Error;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

const WHEEL_STEP: i32 = 120;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScrollError {
    #[error("The scroll area is empty.")]
    EmptyArea,
    #[error("The mouse pointer could not be moved over the scroll area.")]
    Cursor,
    #[error("The mouse-wheel input could not be sent.")]
    Input,
    #[error("The scroll needs at least one step.")]
    NoSteps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Down,
    Up,
}

impl ScrollDirection {
    const fn wheel_delta(self) -> i32 {
        match self {
            Self::Down => -WHEEL_STEP,
            Self::Up => WHEEL_STEP,
        }
    }
}

pub fn scroll(area: Rect, direction: ScrollDirection, notches: u32) -> Result<(), ScrollError> {
    if area.is_empty() {
        return Err(ScrollError::EmptyArea);
    }
    let notches = i32::try_from(notches).map_err(|_| ScrollError::NoSteps)?;
    if notches == 0 {
        return Err(ScrollError::NoSteps);
    }
    let center = area.center();
    // SAFETY: the coordinates are plain pixel values taken from the scroll area centre.
    unsafe { SetCursorPos(center.x.get(), center.y.get()) }.map_err(|_| ScrollError::Cursor)?;
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: direction.wheel_delta().wrapping_mul(notches) as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // SAFETY: the array holds one fully initialised INPUT and the size is that of one element.
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(ScrollError::Input);
    }
    Ok(())
}

pub fn escape_pressed() -> bool {
    // SAFETY: GetAsyncKeyState takes a constant virtual-key code and reads this thread's key state.
    let state = unsafe { GetAsyncKeyState(i32::from(VK_ESCAPE.0)) };
    (state as u16 & 0x8000) != 0
}

pub fn cancelled() -> bool {
    escape_pressed()
}

#[cfg(test)]
mod tests {
    use snapline_core::geometry::{Point, Px};

    use super::*;

    fn area() -> Rect {
        Rect::new(Px(10), Px(20), Px(200), Px(100))
    }

    #[test]
    fn a_notch_scrolls_in_the_expected_direction() {
        assert_eq!(ScrollDirection::Down.wheel_delta(), -120);
        assert_eq!(ScrollDirection::Up.wheel_delta(), 120);
    }

    #[test]
    fn the_scroll_point_is_the_centre_of_the_area() {
        assert_eq!(area().center(), Point::new(Px(110), Px(70)));
    }

    #[test]
    fn an_empty_area_and_zero_notches_are_rejected() {
        assert_eq!(
            scroll(Rect::ZERO, ScrollDirection::Down, 1),
            Err(ScrollError::EmptyArea)
        );
        assert_eq!(
            scroll(area(), ScrollDirection::Down, 0),
            Err(ScrollError::NoSteps)
        );
    }
}
