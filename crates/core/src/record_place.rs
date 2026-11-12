use thiserror::Error;

use crate::geometry::{Px, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBounds {
    /// Which graphics adapter drives the screen; FFmpeg's screen capture only opens adapter 0.
    pub adapter: u32,
    pub index: u32,
    pub bounds: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub output_index: u32,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PlaceError {
    #[error("The recording area is empty.")]
    Empty,
    #[error("The recording area does not fall on any screen.")]
    OffScreen,
    #[error("The recording area spans more than one screen. Choose an area on a single screen.")]
    SpansScreens,
    #[error("This screen is driven by a different graphics adapter, which screen recording cannot capture yet. Record a screen on the main adapter instead.")]
    OtherAdapter,
    #[error("The recording area is too small to record.")]
    TooSmall,
}

/// Spill onto another screen above one part in this many of the area counts as spanning.
const SPAN_LIMIT_DENOMINATOR: i64 = 20;

pub fn place(rect: Rect, outputs: &[OutputBounds]) -> Result<Placement, PlaceError> {
    let rect = rect.normalized();
    if rect.is_empty() {
        return Err(PlaceError::Empty);
    }
    let mut best: Option<(&OutputBounds, Rect)> = None;
    let mut total_overlap = 0_i64;
    for output in outputs {
        if let Some(overlap) = rect.intersection(output.bounds) {
            total_overlap += overlap.area();
            let better = best
                .as_ref()
                .is_none_or(|(_, current)| overlap.area() > current.area());
            if better {
                best = Some((output, overlap));
            }
        }
    }
    let Some((output, overlap)) = best else {
        return Err(PlaceError::OffScreen);
    };
    // A sliver on a neighbouring screen (a window's invisible border, say) does not count as
    // spanning; a real share of the area on a second screen does.
    let spill = total_overlap - overlap.area();
    if spill * SPAN_LIMIT_DENOMINATOR > rect.area() {
        return Err(PlaceError::SpansScreens);
    }
    if output.adapter != 0 {
        return Err(PlaceError::OtherAdapter);
    }
    // Anything hanging off the desktop is cropped to the screen it mostly sits on.
    let rect = overlap;
    let width = rect.width.get() & !1;
    let height = rect.height.get() & !1;
    if width <= 0 || height <= 0 {
        return Err(PlaceError::TooSmall);
    }
    Ok(Placement {
        output_index: output.index,
        rect: Rect::new(
            Px(rect.x.get() - output.bounds.x.get()),
            Px(rect.y.get() - output.bounds.y.get()),
            Px(width),
            Px(height),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn desk() -> Vec<OutputBounds> {
        vec![
            OutputBounds {
                adapter: 0,
                index: 0,
                bounds: rect(0, 0, 1920, 1080),
            },
            OutputBounds {
                adapter: 0,
                index: 1,
                bounds: rect(-1920, 0, 1920, 1080),
            },
            OutputBounds {
                adapter: 0,
                index: 2,
                bounds: rect(1920, 0, 1920, 1080),
            },
            OutputBounds {
                adapter: 0,
                index: 3,
                bounds: rect(0, -1440, 2560, 1440),
            },
            OutputBounds {
                adapter: 0,
                index: 4,
                bounds: rect(3840, -300, 3840, 2160),
            },
        ]
    }

    #[test]
    fn the_primary_output_keeps_its_offsets() {
        let placed = place(rect(100, 50, 640, 360), &desk()).expect("place");
        assert_eq!(placed.output_index, 0);
        assert_eq!(placed.rect, rect(100, 50, 640, 360));
    }

    #[test]
    fn a_screen_to_the_right_uses_offsets_relative_to_that_output() {
        let placed = place(rect(2020, 100, 800, 600), &desk()).expect("place");
        assert_eq!(placed.output_index, 2);
        assert_eq!(placed.rect, rect(100, 100, 800, 600));
    }

    #[test]
    fn a_screen_to_the_left_has_no_negative_offsets() {
        let placed = place(rect(-1800, 20, 1000, 500), &desk()).expect("place");
        assert_eq!(placed.output_index, 1);
        assert_eq!(placed.rect, rect(120, 20, 1000, 500));
    }

    #[test]
    fn a_screen_above_has_no_negative_offsets() {
        let placed = place(rect(300, -1000, 800, 600), &desk()).expect("place");
        assert_eq!(placed.output_index, 3);
        assert_eq!(placed.rect, rect(300, 440, 800, 600));
    }

    #[test]
    fn a_high_dpi_screen_uses_physical_pixels() {
        let placed = place(rect(3840, -300, 3840, 2160), &desk()).expect("place");
        assert_eq!(placed.output_index, 4);
        assert_eq!(placed.rect, rect(0, 0, 3840, 2160));
    }

    #[test]
    fn odd_sizes_are_rounded_down_to_even() {
        let placed = place(rect(11, 21, 641, 361), &desk()).expect("place");
        assert_eq!(placed.rect, rect(11, 21, 640, 360));
    }

    #[test]
    fn an_area_spanning_two_screens_is_rejected() {
        assert_eq!(
            place(rect(1800, 100, 400, 300), &desk()),
            Err(PlaceError::SpansScreens)
        );
    }

    #[test]
    fn a_window_border_spilling_onto_a_neighbour_is_cropped_not_rejected() {
        let placed = place(rect(1912, -8, 1936, 1096), &desk()).expect("place");
        assert_eq!(placed.output_index, 2);
        assert_eq!(placed.rect, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn an_area_hanging_off_the_desktop_is_cropped_to_its_screen() {
        let placed = place(rect(-8, -8, 1936, 1096), &desk()).expect("place");
        assert_eq!(placed.output_index, 0);
        assert_eq!(placed.rect, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn a_screen_on_another_adapter_gets_a_clear_message() {
        let mut outputs = desk();
        outputs[2].adapter = 1;
        assert_eq!(
            place(rect(2020, 100, 800, 600), &outputs),
            Err(PlaceError::OtherAdapter)
        );
    }

    #[test]
    fn an_area_off_every_screen_is_rejected() {
        assert_eq!(
            place(rect(9000, 9000, 100, 100), &desk()),
            Err(PlaceError::OffScreen)
        );
    }

    #[test]
    fn empty_and_one_pixel_areas_are_rejected() {
        assert_eq!(place(Rect::ZERO, &desk()), Err(PlaceError::Empty));
        assert_eq!(
            place(rect(10, 10, 1, 1), &desk()),
            Err(PlaceError::TooSmall)
        );
    }
}
