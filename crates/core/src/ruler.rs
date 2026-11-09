use serde::{Deserialize, Serialize};

use crate::geometry::{DpiScale, Point, Px, Rect, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RulerAxis {
    Horizontal,
    Vertical,
}

impl RulerAxis {
    pub fn dominant(start: Point, end: Point) -> Self {
        let dx = delta(start.x, end.x).abs();
        let dy = delta(start.y, end.y).abs();
        if dx >= dy {
            Self::Horizontal
        } else {
            Self::Vertical
        }
    }

    pub const fn lock(self, start: Point, end: Point) -> Point {
        match self {
            Self::Horizontal => Point::new(end.x, start.y),
            Self::Vertical => Point::new(start.x, end.y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RulerMeasurement {
    pub start: Point,
    pub end: Point,
    pub width: Px,
    pub height: Px,
    pub axis: Option<RulerAxis>,
}

impl RulerMeasurement {
    pub fn measure(start: Point, end: Point) -> Self {
        Self {
            start,
            end,
            width: span(start.x, end.x),
            height: span(start.y, end.y),
            axis: None,
        }
    }

    pub fn locked(start: Point, end: Point) -> Self {
        let axis = RulerAxis::dominant(start, end);
        Self::measure_axis(start, axis.lock(start, end), axis)
    }

    pub fn measure_axis(start: Point, end: Point, axis: RulerAxis) -> Self {
        Self {
            start,
            end,
            width: span(start.x, end.x),
            height: span(start.y, end.y),
            axis: Some(axis),
        }
    }

    pub fn measure_with_lock(start: Point, end: Point, lock: bool) -> Self {
        if lock {
            Self::locked(start, end)
        } else {
            Self::measure(start, end)
        }
    }

    pub fn delta_x(self) -> i64 {
        delta(self.start.x, self.end.x)
    }

    pub fn delta_y(self) -> i64 {
        delta(self.start.y, self.end.y)
    }

    pub fn diagonal(self) -> f64 {
        (self.delta_x() as f64).hypot(self.delta_y() as f64)
    }

    pub fn angle_degrees(self) -> f64 {
        let dx = self.delta_x();
        let dy = self.delta_y();
        if dx == 0 && dy == 0 {
            0.0
        } else {
            (dy as f64).atan2(dx as f64).to_degrees()
        }
    }

    pub fn rect(self) -> Rect {
        Rect::new(
            Px(self.start.x.get().min(self.end.x.get())),
            Px(self.start.y.get().min(self.end.y.get())),
            self.width,
            self.height,
        )
    }

    pub fn is_empty(self) -> bool {
        self.width == Px::ZERO && self.height == Px::ZERO
    }

    pub fn size_text(self) -> String {
        format!("{} × {}", self.width.get(), self.height.get())
    }

    pub fn copy_text(self) -> String {
        self.size_text()
    }

    pub fn logical_size(self, dpi: DpiScale) -> Size {
        dpi.physical_size_to_logical(Size::new(self.width, self.height))
    }

    pub fn logical_diagonal(self, dpi: DpiScale) -> f64 {
        let size = self.logical_size(dpi);
        (size.width.get() as f64).hypot(size.height.get() as f64)
    }
}

fn delta(a: Px, b: Px) -> i64 {
    i64::from(b.get()) - i64::from(a.get())
}

fn span(a: Px, b: Px) -> Px {
    Px(i32::try_from(delta(a, b).abs()).unwrap_or(i32::MAX))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    #[test]
    fn width_height_and_diagonal_ignore_drag_direction() {
        let expected = RulerMeasurement {
            start: point(100, 100),
            end: point(400, 500),
            width: Px(300),
            height: Px(400),
            axis: None,
        };
        for (start, end) in [
            (point(100, 100), point(400, 500)),
            (point(400, 100), point(100, 500)),
            (point(100, 500), point(400, 100)),
            (point(400, 500), point(100, 100)),
        ] {
            let measurement = RulerMeasurement::measure(start, end);
            assert_eq!(measurement.width, expected.width);
            assert_eq!(measurement.height, expected.height);
            assert_eq!(
                measurement.rect(),
                Rect::new(Px(100), Px(100), Px(300), Px(400))
            );
            assert!((measurement.diagonal() - 500.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn a_zero_length_drag_measures_zero_at_zero_degrees() {
        let measurement = RulerMeasurement::measure(point(512, 256), point(512, 256));
        assert_eq!(measurement.width, Px::ZERO);
        assert_eq!(measurement.height, Px::ZERO);
        assert_eq!(measurement.diagonal(), 0.0);
        assert_eq!(measurement.angle_degrees(), 0.0);
        assert!(measurement.is_empty());
        assert_eq!(measurement.size_text(), "0 × 0");
        assert_eq!(measurement.copy_text(), "0 × 0");
    }

    #[test]
    fn the_angle_is_signed_from_the_horizontal() {
        let origin = point(0, 0);
        assert_eq!(
            RulerMeasurement::measure(origin, point(10, 0)).angle_degrees(),
            0.0
        );
        assert_eq!(
            RulerMeasurement::measure(origin, point(0, 10)).angle_degrees(),
            90.0
        );
        assert_eq!(
            RulerMeasurement::measure(origin, point(-10, 0)).angle_degrees(),
            180.0
        );
        assert_eq!(
            RulerMeasurement::measure(origin, point(0, -10)).angle_degrees(),
            -90.0
        );
        assert!(
            (RulerMeasurement::measure(origin, point(-10, -10)).angle_degrees() + 135.0).abs()
                < 1e-9
        );
    }

    #[test]
    fn shift_locks_to_the_dominant_axis() {
        let start = point(100, 100);
        let horizontal = RulerMeasurement::locked(start, point(400, 160));
        assert_eq!(horizontal.end, point(400, 100));
        assert_eq!(horizontal.height, Px::ZERO);
        assert_eq!(horizontal.axis, Some(RulerAxis::Horizontal));
        assert_eq!(horizontal.angle_degrees(), 0.0);

        let vertical = RulerMeasurement::locked(start, point(160, 500));
        assert_eq!(vertical.end, point(100, 500));
        assert_eq!(vertical.width, Px::ZERO);
        assert_eq!(vertical.axis, Some(RulerAxis::Vertical));
        assert_eq!(vertical.angle_degrees(), 90.0);
    }

    #[test]
    fn an_equal_drag_prefers_the_horizontal_lock() {
        let measurement = RulerMeasurement::locked(point(0, 0), point(40, 40));
        assert_eq!(measurement.axis, Some(RulerAxis::Horizontal));
        assert_eq!(measurement.end, point(40, 0));
    }

    #[test]
    fn the_lock_can_be_switched_off_again() {
        let start = point(10, 10);
        let end = point(90, 40);
        assert_eq!(
            RulerMeasurement::measure_with_lock(start, end, false),
            RulerMeasurement::measure(start, end)
        );
        assert_eq!(
            RulerMeasurement::measure_with_lock(start, end, true),
            RulerMeasurement::locked(start, end)
        );
    }

    #[test]
    fn multi_monitor_coordinates_keep_their_sign() {
        let measurement = RulerMeasurement::measure(point(-1920, -200), point(-1280, 100));
        assert_eq!(measurement.width, Px(640));
        assert_eq!(measurement.height, Px(300));
        assert_eq!(
            measurement.rect(),
            Rect::new(Px(-1920), Px(-200), Px(640), Px(300))
        );
        assert_eq!(measurement.delta_x(), 640);
        assert_eq!(measurement.delta_y(), 300);
        let reversed = RulerMeasurement::measure(point(-1280, 100), point(-1920, -200));
        assert_eq!(reversed.width, Px(640));
        assert_eq!(reversed.height, Px(300));
    }

    #[test]
    fn coordinates_across_the_signed_range_do_not_overflow() {
        let measurement = RulerMeasurement::measure(point(i32::MIN, 0), point(i32::MAX, 0));
        assert_eq!(measurement.width, Px(i32::MAX));
        assert_eq!(measurement.delta_x(), i64::from(u32::MAX));
    }

    #[test]
    fn the_readout_uses_physical_pixels_and_the_multiplication_sign() {
        let measurement = RulerMeasurement::measure(point(0, 0), point(660, 500));
        assert_eq!(measurement.size_text(), "660 × 500");
        assert_eq!(measurement.copy_text(), "660 × 500");
    }

    #[test]
    fn dpi_converts_the_measurement_for_display_only() {
        let measurement = RulerMeasurement::measure(point(0, 0), point(400, 300));
        let dpi = DpiScale::from_percent(200);
        assert_eq!(measurement.logical_size(dpi), Size::new(Px(200), Px(150)));
        assert!((measurement.logical_diagonal(dpi) - 250.0).abs() < 1e-9);
        assert_eq!(measurement.diagonal(), 500.0);
    }

    proptest! {
        #[test]
        fn any_drag_has_non_negative_spans(
            ax in -100_000i32..100_000,
            ay in -100_000i32..100_000,
            bx in -100_000i32..100_000,
            by in -100_000i32..100_000,
        ) {
            let measurement = RulerMeasurement::measure(point(ax, ay), point(bx, by));
            prop_assert!(measurement.width.get() >= 0);
            prop_assert!(measurement.height.get() >= 0);
            prop_assert!(measurement.diagonal() >= 0.0);
            let rect = measurement.rect();
            prop_assert_eq!(rect.width, measurement.width);
            prop_assert_eq!(rect.height, measurement.height);
            prop_assert!(rect.width.get() >= 0);
            prop_assert!(rect.height.get() >= 0);
        }

        #[test]
        fn locking_always_removes_one_span(
            ax in -100_000i32..100_000,
            ay in -100_000i32..100_000,
            bx in -100_000i32..100_000,
            by in -100_000i32..100_000,
        ) {
            let measurement = RulerMeasurement::locked(point(ax, ay), point(bx, by));
            prop_assert!(measurement.width.get() == 0 || measurement.height.get() == 0);
        }
    }
}
