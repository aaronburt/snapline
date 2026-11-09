use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Px(pub i32);

impl Px {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);

    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> i32 {
        self.0
    }

    pub const fn abs(self) -> Self {
        Self(self.0.abs())
    }

    pub const fn min(self, other: Self) -> Self {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    pub const fn max(self, other: Self) -> Self {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    pub const fn clamp(self, min: Self, max: Self) -> Self {
        if self.0 < min.0 {
            min
        } else if self.0 > max.0 {
            max
        } else {
            self
        }
    }
}

impl From<i32> for Px {
    fn from(value: i32) -> Self {
        Self(value)
    }
}

impl From<Px> for i32 {
    fn from(value: Px) -> Self {
        value.0
    }
}

impl std::ops::Add for Px {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for Px {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl std::ops::Sub for Px {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl std::ops::SubAssign for Px {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl std::ops::Neg for Px {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl std::ops::Mul<i32> for Px {
    type Output = Self;

    fn mul(self, rhs: i32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl std::ops::Div<i32> for Px {
    type Output = Self;

    fn div(self, rhs: i32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Point {
    pub x: Px,
    pub y: Px,
}

impl Point {
    pub const ORIGIN: Self = Self::new(Px::ZERO, Px::ZERO);

    pub const fn new(x: Px, y: Px) -> Self {
        Self { x, y }
    }

    pub const fn offset(self, dx: Px, dy: Px) -> Self {
        Self::new(Px(self.x.0 + dx.0), Px(self.y.0 + dy.0))
    }
}

impl From<(i32, i32)> for Point {
    fn from((x, y): (i32, i32)) -> Self {
        Self::new(Px(x), Px(y))
    }
}

impl std::ops::Add for Point {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Point {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl std::ops::Neg for Point {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Size {
    pub width: Px,
    pub height: Px,
}

impl Size {
    pub const ZERO: Self = Self::new(Px::ZERO, Px::ZERO);

    pub const fn new(width: Px, height: Px) -> Self {
        Self { width, height }
    }

    pub const fn is_empty(self) -> bool {
        self.width.0 <= 0 || self.height.0 <= 0
    }

    pub const fn area(self) -> i64 {
        self.width.0 as i64 * self.height.0 as i64
    }
}

impl std::ops::Mul<i32> for Size {
    type Output = Self;

    fn mul(self, rhs: i32) -> Self::Output {
        Self::new(self.width * rhs, self.height * rhs)
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Rect {
    pub x: Px,
    pub y: Px,
    pub width: Px,
    pub height: Px,
}

impl Rect {
    pub const ZERO: Self = Self::new(Px::ZERO, Px::ZERO, Px::ZERO, Px::ZERO);

    pub const fn new(x: Px, y: Px, width: Px, height: Px) -> Self {
        if width.0 < 0 {
            if height.0 < 0 {
                Self {
                    x: Px(x.0 + width.0),
                    y: Px(y.0 + height.0),
                    width: Px(-width.0),
                    height: Px(-height.0),
                }
            } else {
                Self {
                    x: Px(x.0 + width.0),
                    y,
                    width: Px(-width.0),
                    height,
                }
            }
        } else if height.0 < 0 {
            Self {
                x,
                y: Px(y.0 + height.0),
                width,
                height: Px(-height.0),
            }
        } else {
            Self {
                x,
                y,
                width,
                height,
            }
        }
    }

    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    pub const fn from_points(a: Point, b: Point) -> Self {
        let x = if a.x.0 <= b.x.0 { a.x } else { b.x };
        let y = if a.y.0 <= b.y.0 { a.y } else { b.y };
        let width = Px((a.x.0 - b.x.0).abs());
        let height = Px((a.y.0 - b.y.0).abs());
        Self::new(x, y, width, height)
    }

    pub const fn from_edges(left: Px, top: Px, right: Px, bottom: Px) -> Self {
        Self::new(left, top, Px(right.0 - left.0), Px(bottom.0 - top.0))
    }

    pub const fn left(self) -> Px {
        self.x
    }

    pub const fn top(self) -> Px {
        self.y
    }

    pub const fn right(self) -> Px {
        Px(self.x.0 + self.width.0)
    }

    pub const fn bottom(self) -> Px {
        Px(self.y.0 + self.height.0)
    }

    pub const fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }

    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    pub const fn center(self) -> Point {
        Point::new(
            Px(self.x.0 + self.width.0 / 2),
            Px(self.y.0 + self.height.0 / 2),
        )
    }

    pub const fn is_empty(self) -> bool {
        self.width.0 <= 0 || self.height.0 <= 0
    }

    pub const fn area(self) -> i64 {
        self.width.0 as i64 * self.height.0 as i64
    }

    pub const fn normalized(self) -> Self {
        self
    }

    pub const fn translated(self, delta: Point) -> Self {
        Self::new(
            Px(self.x.0 + delta.x.0),
            Px(self.y.0 + delta.y.0),
            self.width,
            self.height,
        )
    }

    pub const fn contains(self, point: Point) -> bool {
        point.x.0 >= self.x.0
            && point.y.0 >= self.y.0
            && point.x.0 < self.x.0 + self.width.0
            && point.y.0 < self.y.0 + self.height.0
    }

    pub const fn contains_rect(self, other: Self) -> bool {
        other.x.0 >= self.x.0
            && other.y.0 >= self.y.0
            && other.x.0 + other.width.0 <= self.x.0 + self.width.0
            && other.y.0 + other.height.0 <= self.y.0 + self.height.0
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right.0 <= left.0 || bottom.0 <= top.0 {
            None
        } else {
            Some(Self::from_edges(left, top, right, bottom))
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self::from_edges(
            self.left().min(other.left()),
            self.top().min(other.top()),
            self.right().max(other.right()),
            self.bottom().max(other.bottom()),
        )
    }

    pub fn clamp(self, bounds: Self) -> Self {
        let width = self.width.min(bounds.width).max(Px::ZERO);
        let height = self.height.min(bounds.height).max(Px::ZERO);
        let max_x = bounds.right() - width;
        let max_y = bounds.bottom() - height;
        let x = self.x.max(bounds.left()).min(max_x);
        let y = self.y.max(bounds.top()).min(max_y);
        Self::new(x, y, width, height)
    }

    pub const fn top_left(self) -> Point {
        Point::new(self.x, self.y)
    }

    pub const fn top_right(self) -> Point {
        Point::new(self.right(), self.y)
    }

    pub const fn bottom_left(self) -> Point {
        Point::new(self.x, self.bottom())
    }

    pub const fn bottom_right(self) -> Point {
        Point::new(self.right(), self.bottom())
    }

    pub const fn top_center(self) -> Point {
        Point::new(Px(self.x.0 + self.width.0 / 2), self.y)
    }

    pub const fn bottom_center(self) -> Point {
        Point::new(Px(self.x.0 + self.width.0 / 2), self.bottom())
    }

    pub const fn left_center(self) -> Point {
        Point::new(self.x, Px(self.y.0 + self.height.0 / 2))
    }

    pub const fn right_center(self) -> Point {
        Point::new(self.right(), Px(self.y.0 + self.height.0 / 2))
    }

    pub const fn corners(self) -> [Point; 4] {
        [
            self.top_left(),
            self.top_right(),
            self.bottom_right(),
            self.bottom_left(),
        ]
    }

    pub const fn edges(self) -> [Point; 4] {
        [
            self.top_center(),
            self.right_center(),
            self.bottom_center(),
            self.left_center(),
        ]
    }

    pub const fn handles(self) -> [Point; 8] {
        [
            Grip::NorthWest.point(self),
            Grip::North.point(self),
            Grip::NorthEast.point(self),
            Grip::East.point(self),
            Grip::SouthEast.point(self),
            Grip::South.point(self),
            Grip::SouthWest.point(self),
            Grip::West.point(self),
        ]
    }

    pub fn grip_at(self, point: Point) -> Option<Grip> {
        let half = Px(HANDLE_SIZE.0 / 2);
        GRIPS
            .iter()
            .filter_map(|grip| {
                let center = grip.point(self);
                let dx = (point.x - center.x).abs();
                let dy = (point.y - center.y).abs();
                if dx.0 <= half.0 && dy.0 <= half.0 {
                    let distance =
                        i64::from(dx.0) * i64::from(dx.0) + i64::from(dy.0) * i64::from(dy.0);
                    Some((*grip, distance))
                } else {
                    None
                }
            })
            .min_by_key(|(_, distance)| *distance)
            .map(|(grip, _)| grip)
    }
}

pub const HANDLE_SIZE: Px = Px(9);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Grip {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    Body,
}

pub const GRIPS: [Grip; 8] = [
    Grip::NorthWest,
    Grip::North,
    Grip::NorthEast,
    Grip::East,
    Grip::SouthEast,
    Grip::South,
    Grip::SouthWest,
    Grip::West,
];

impl Grip {
    pub const fn is_resize(self) -> bool {
        !matches!(self, Self::Body)
    }

    pub const fn point(self, rect: Rect) -> Point {
        match self {
            Self::NorthWest => rect.top_left(),
            Self::North => rect.top_center(),
            Self::NorthEast => rect.top_right(),
            Self::East => rect.right_center(),
            Self::SouthEast => rect.bottom_right(),
            Self::South => rect.bottom_center(),
            Self::SouthWest => rect.bottom_left(),
            Self::West => rect.left_center(),
            Self::Body => rect.center(),
        }
    }

    pub const fn resize_to(self, rect: Rect, point: Point) -> Rect {
        let mut left = rect.left();
        let mut top = rect.top();
        let mut right = rect.right();
        let mut bottom = rect.bottom();
        match self {
            Self::NorthWest => {
                left = point.x;
                top = point.y;
            }
            Self::North => top = point.y,
            Self::NorthEast => {
                right = point.x;
                top = point.y;
            }
            Self::East => right = point.x,
            Self::SouthEast => {
                right = point.x;
                bottom = point.y;
            }
            Self::South => bottom = point.y,
            Self::SouthWest => {
                left = point.x;
                bottom = point.y;
            }
            Self::West => left = point.x,
            Self::Body => {}
        }
        Rect::from_edges(left, top, right, bottom)
    }

    pub fn nudge(self, rect: Rect, direction: Direction, amount: Px) -> Rect {
        let delta = direction.delta(amount);
        let left = rect.left();
        let top = rect.top();
        let right = rect.right();
        let bottom = rect.bottom();
        match self {
            Self::NorthWest => Rect::from_edges(left + delta.x, top + delta.y, right, bottom),
            Self::North => Rect::from_edges(left, top + delta.y, right, bottom),
            Self::NorthEast => Rect::from_edges(left, top + delta.y, right + delta.x, bottom),
            Self::East => Rect::from_edges(left, top, right + delta.x, bottom),
            Self::SouthEast => Rect::from_edges(left, top, right + delta.x, bottom + delta.y),
            Self::South => Rect::from_edges(left, top, right, bottom + delta.y),
            Self::SouthWest => Rect::from_edges(left + delta.x, top, right, bottom + delta.y),
            Self::West => Rect::from_edges(left + delta.x, top, right, bottom),
            Self::Body => rect.translated(delta),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub const fn delta(self, amount: Px) -> Point {
        match self {
            Self::Left => Point::new(Px(-amount.0), Px::ZERO),
            Self::Right => Point::new(amount, Px::ZERO),
            Self::Up => Point::new(Px::ZERO, Px(-amount.0)),
            Self::Down => Point::new(Px::ZERO, amount),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DpiScale(f32);

impl DpiScale {
    pub const ONE: Self = Self(1.0);

    pub fn new(scale: f32) -> Self {
        if scale.is_finite() && scale > 0.0 {
            Self(scale)
        } else {
            Self::ONE
        }
    }

    pub fn from_dpi(dpi: u32) -> Self {
        Self::new(dpi as f32 / 96.0)
    }

    pub fn from_percent(percent: u32) -> Self {
        Self::new(percent as f32 / 100.0)
    }

    pub const fn scale(self) -> f32 {
        self.0
    }

    pub fn physical_to_logical(self, px: Px) -> Px {
        Px((px.0 as f32 / self.0).round() as i32)
    }

    pub fn logical_to_physical(self, px: Px) -> Px {
        Px((px.0 as f32 * self.0).round() as i32)
    }

    pub fn physical_point_to_logical(self, point: Point) -> Point {
        Point::new(
            self.physical_to_logical(point.x),
            self.physical_to_logical(point.y),
        )
    }

    pub fn logical_point_to_physical(self, point: Point) -> Point {
        Point::new(
            self.logical_to_physical(point.x),
            self.logical_to_physical(point.y),
        )
    }

    pub fn physical_size_to_logical(self, size: Size) -> Size {
        Size::new(
            self.physical_to_logical(size.width),
            self.physical_to_logical(size.height),
        )
    }

    pub fn logical_size_to_physical(self, size: Size) -> Size {
        Size::new(
            self.logical_to_physical(size.width),
            self.logical_to_physical(size.height),
        )
    }

    pub fn physical_rect_to_logical(self, rect: Rect) -> Rect {
        Rect::from_origin_size(
            self.physical_point_to_logical(rect.origin()),
            self.physical_size_to_logical(rect.size()),
        )
    }

    pub fn logical_rect_to_physical(self, rect: Rect) -> Rect {
        Rect::from_origin_size(
            self.logical_point_to_physical(rect.origin()),
            self.logical_size_to_physical(rect.size()),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MonitorGeometry {
    pub bounds: Rect,
    pub work_area: Rect,
    pub dpi: DpiScale,
}

impl MonitorGeometry {
    pub const fn new(bounds: Rect, work_area: Rect, dpi: DpiScale) -> Self {
        Self {
            bounds,
            work_area,
            dpi,
        }
    }

    pub const fn contains(self, point: Point) -> bool {
        self.bounds.contains(point)
    }

    pub fn logical_bounds(self) -> Rect {
        self.dpi.physical_rect_to_logical(self.bounds)
    }

    pub fn logical_work_area(self) -> Rect {
        self.dpi.physical_rect_to_logical(self.work_area)
    }

    pub fn physical_rect_to_logical(self, rect: Rect) -> Rect {
        self.dpi.physical_rect_to_logical(rect)
    }

    pub fn logical_rect_to_physical(self, rect: Rect) -> Rect {
        self.dpi.logical_rect_to_physical(rect)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    #[test]
    fn px_arithmetic() {
        assert_eq!(Px(3) + Px(4), Px(7));
        assert_eq!(Px(3) - Px(4), Px(-1));
        assert_eq!(-Px(3), Px(-3));
        assert_eq!(Px(3) * 4, Px(12));
        assert_eq!(Px(12) / 4, Px(3));
        assert_eq!(Px(-3).abs(), Px(3));
    }

    #[test]
    fn rect_new_normalises_negative_sizes() {
        assert_eq!(rect(10, 20, -5, -8), rect(5, 12, 5, 8));
        assert_eq!(rect(10, 20, -5, 8), rect(5, 20, 5, 8));
        assert_eq!(rect(10, 20, 5, -8), rect(10, 12, 5, 8));
    }

    #[test]
    fn rect_from_points_works_from_any_corner() {
        let expected = rect(10, 20, 30, 40);
        assert_eq!(Rect::from_points(point(10, 20), point(40, 60)), expected);
        assert_eq!(Rect::from_points(point(40, 20), point(10, 60)), expected);
        assert_eq!(Rect::from_points(point(10, 60), point(40, 20)), expected);
        assert_eq!(Rect::from_points(point(40, 60), point(10, 20)), expected);
        assert_eq!(
            Rect::from_points(point(25, 40), point(25, 40)).is_empty(),
            true
        );
    }

    #[test]
    fn rect_edges_and_corners() {
        let r = rect(10, 20, 30, 40);
        assert_eq!(r.left(), Px(10));
        assert_eq!(r.top(), Px(20));
        assert_eq!(r.right(), Px(40));
        assert_eq!(r.bottom(), Px(60));
        assert_eq!(r.origin(), point(10, 20));
        assert_eq!(r.size(), Size::new(Px(30), Px(40)));
        assert_eq!(r.center(), point(25, 40));
        assert_eq!(r.corners()[0], point(10, 20));
        assert_eq!(r.corners()[2], point(40, 60));
        assert_eq!(r.edges()[0], point(25, 20));
        assert_eq!(r.handles().len(), 8);
    }

    #[test]
    fn rect_contains_and_intersection() {
        let r = rect(10, 20, 30, 40);
        assert!(r.contains(point(10, 20)));
        assert!(r.contains(point(39, 59)));
        assert!(!r.contains(point(40, 60)));
        assert!(r.contains_rect(rect(12, 22, 5, 5)));
        assert!(!r.contains_rect(rect(5, 20, 10, 10)));
        assert_eq!(
            r.intersection(rect(20, 30, 100, 100)),
            Some(rect(20, 30, 20, 30))
        );
        assert_eq!(r.intersection(rect(50, 50, 10, 10)), None);
        assert_eq!(r.union(rect(50, 50, 10, 10)), rect(10, 20, 50, 40));
    }

    #[test]
    fn rect_clamp_keeps_it_inside_bounds() {
        let bounds = rect(0, 0, 100, 100);
        assert_eq!(rect(90, 90, 20, 20).clamp(bounds), rect(80, 80, 20, 20));
        assert_eq!(rect(-5, -5, 20, 20).clamp(bounds), rect(0, 0, 20, 20));
        assert_eq!(rect(10, 10, 200, 200).clamp(bounds), rect(0, 0, 100, 100));
    }

    #[test]
    fn grip_hit_test_finds_each_handle() {
        let r = rect(100, 200, 300, 400);
        for grip in GRIPS {
            assert_eq!(r.grip_at(grip.point(r)), Some(grip));
        }
        assert_eq!(r.grip_at(r.center()), None);
        assert!(Grip::Body.is_resize() == false);
    }

    #[test]
    fn every_grip_resizes_and_normalises() {
        let r = rect(10, 20, 100, 200);
        assert_eq!(
            Grip::NorthWest.resize_to(r, point(0, 0)),
            rect(0, 0, 110, 220)
        );
        assert_eq!(
            Grip::North.resize_to(r, point(50, 0)),
            rect(10, 0, 100, 220)
        );
        assert_eq!(
            Grip::NorthEast.resize_to(r, point(200, 0)),
            rect(10, 0, 190, 220)
        );
        assert_eq!(
            Grip::East.resize_to(r, point(200, 50)),
            rect(10, 20, 190, 200)
        );
        assert_eq!(
            Grip::SouthEast.resize_to(r, point(200, 300)),
            rect(10, 20, 190, 280)
        );
        assert_eq!(
            Grip::South.resize_to(r, point(50, 300)),
            rect(10, 20, 100, 280)
        );
        assert_eq!(
            Grip::SouthWest.resize_to(r, point(0, 300)),
            rect(0, 20, 110, 280)
        );
        assert_eq!(Grip::West.resize_to(r, point(0, 50)), rect(0, 20, 110, 200));
        assert_eq!(
            Grip::West.resize_to(r, point(500, 50)),
            rect(110, 20, 390, 200)
        );
        assert_eq!(
            Grip::NorthWest.resize_to(r, point(200, 300)),
            rect(110, 220, 90, 80)
        );
    }

    #[test]
    fn grip_nudge_moves_the_expected_edges() {
        let r = rect(10, 20, 100, 200);
        assert_eq!(
            Grip::West.nudge(r, Direction::Left, Px(5)),
            rect(5, 20, 105, 200)
        );
        assert_eq!(
            Grip::East.nudge(r, Direction::Right, Px(5)),
            rect(10, 20, 105, 200)
        );
        assert_eq!(
            Grip::North.nudge(r, Direction::Up, Px(5)),
            rect(10, 15, 100, 205)
        );
        assert_eq!(
            Grip::South.nudge(r, Direction::Down, Px(5)),
            rect(10, 20, 100, 205)
        );
        assert_eq!(
            Grip::SouthEast.nudge(r, Direction::Left, Px(5)),
            rect(10, 20, 95, 200)
        );
        assert_eq!(
            Grip::Body.nudge(r, Direction::Right, Px(5)),
            rect(15, 20, 100, 200)
        );
        assert_eq!(
            Grip::NorthWest.nudge(r, Direction::Down, Px(300)),
            rect(10, 220, 100, 100)
        );
    }

    #[test]
    fn dpi_conversions_round_trip() {
        for scale in [
            DpiScale::from_percent(100),
            DpiScale::from_percent(150),
            DpiScale::from_percent(200),
        ] {
            let logical = rect(10, 20, 30, 40);
            let physical = scale.logical_rect_to_physical(logical);
            assert_eq!(scale.physical_rect_to_logical(physical), logical);
        }
        assert_eq!(
            DpiScale::from_dpi(144).logical_to_physical(Px(100)),
            Px(150)
        );
        assert_eq!(
            DpiScale::from_dpi(144).physical_to_logical(Px(150)),
            Px(100)
        );
        assert_eq!(DpiScale::new(0.0), DpiScale::ONE);
    }

    #[test]
    fn monitor_geometry_converts_both_ways() {
        let monitor = MonitorGeometry::new(
            rect(0, 0, 2560, 1440),
            rect(0, 0, 2560, 1400),
            DpiScale::from_percent(200),
        );
        assert!(monitor.contains(point(100, 100)));
        assert_eq!(monitor.logical_bounds(), rect(0, 0, 1280, 720));
        assert_eq!(monitor.logical_work_area(), rect(0, 0, 1280, 700));
        let physical = rect(200, 200, 400, 300);
        assert_eq!(
            monitor.logical_rect_to_physical(monitor.physical_rect_to_logical(physical)),
            physical
        );
    }

    proptest! {
        #[test]
        fn from_points_contains_both_points(
            ax in -10000i32..10000,
            ay in -10000i32..10000,
            bx in -10000i32..10000,
            by in -10000i32..10000,
        ) {
            let a = point(ax, ay);
            let b = point(bx, by);
            let r = Rect::from_points(a, b);
            prop_assert!(r.width.0 >= 0);
            prop_assert!(r.height.0 >= 0);
            for p in [a, b] {
                prop_assert!(p.x.0 >= r.left().0);
                prop_assert!(p.x.0 <= r.right().0);
                prop_assert!(p.y.0 >= r.top().0);
                prop_assert!(p.y.0 <= r.bottom().0);
            }
        }

        #[test]
        fn any_grip_resize_is_normalised_and_never_panics(
            x in -10000i32..10000,
            y in -10000i32..10000,
            w in 0i32..10000,
            h in 0i32..10000,
            px in -10000i32..10000,
            py in -10000i32..10000,
        ) {
            let r = rect(x, y, w, h);
            for grip in GRIPS {
                let resized = grip.resize_to(r, point(px, py));
                prop_assert!(resized.width.0 >= 0);
                prop_assert!(resized.height.0 >= 0);
                let nudged = grip.nudge(r, Direction::Left, Px(1));
                prop_assert!(nudged.width.0 >= 0);
                prop_assert!(nudged.height.0 >= 0);
            }
        }
    }
}
