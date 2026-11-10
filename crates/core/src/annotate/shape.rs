use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Px, Rect, Size};

use super::canvas::Rgba;

pub const STROKE_THIN: Px = Px(2);
pub const STROKE_MEDIUM: Px = Px(4);
pub const STROKE_THICK: Px = Px(8);
pub const DOT_SMALL: Px = Px(4);
pub const DOT_MEDIUM: Px = Px(7);
pub const DOT_LARGE: Px = Px(11);
pub const TEXT_SIZE: Px = Px(16);
pub const STEP_DIAMETER: Px = Px(26);
pub const STEP_RADIUS: Px = Px(13);
pub const STEP_SIZE: Px = Px(13);
pub const CORNER_RADIUS: Px = Px(8);
pub const LABEL_RADIUS: Px = Px(4);
pub const LABEL_PAD_X: Px = Px(6);
pub const LABEL_PAD_Y: Px = Px(2);
pub const BLUR_BLOCK: Px = Px(12);
pub const GLYPH_ADVANCE: Px = Px(6);
pub const GLYPH_HEIGHT: Px = Px(7);
pub const LABEL_BACKGROUND: Rgba = Rgba::new(255, 255, 255, 255);
pub const LABEL_BACKGROUND_DARK: Rgba = Rgba::new(20, 20, 20, 209);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Colour {
    #[default]
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    White,
}

impl Colour {
    pub const ALL: [Self; 6] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::White,
    ];

    pub const fn rgba(self) -> Rgba {
        match self {
            Self::Red => Rgba::new(229, 72, 77, 255),
            Self::Orange => Rgba::new(247, 107, 21, 255),
            Self::Yellow => Rgba::new(255, 214, 10, 255),
            Self::Green => Rgba::new(48, 164, 108, 255),
            Self::Blue => Rgba::new(0, 144, 255, 255),
            Self::White => Rgba::new(255, 255, 255, 255),
        }
    }

    pub const fn highlight(self) -> Rgba {
        match self {
            Self::Red => Rgba::new(229, 72, 77, 82),
            Self::Orange => Rgba::new(247, 107, 21, 82),
            Self::Yellow => Rgba::new(255, 214, 10, 140),
            Self::Green => Rgba::new(48, 164, 108, 82),
            Self::Blue => Rgba::new(0, 144, 255, 71),
            Self::White => Rgba::new(200, 200, 200, 102),
        }
    }

    pub const fn hex(self) -> &'static str {
        match self {
            Self::Red => "#e5484d",
            Self::Orange => "#f76b15",
            Self::Yellow => "#ffd60a",
            Self::Green => "#30a46c",
            Self::Blue => "#0090ff",
            Self::White => "#ffffff",
        }
    }

    pub const fn label_background(self) -> Rgba {
        if self.is_white() {
            LABEL_BACKGROUND_DARK
        } else {
            LABEL_BACKGROUND
        }
    }

    pub const fn is_white(self) -> bool {
        matches!(self, Self::White)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Weight {
    Thin,
    #[default]
    Medium,
    Thick,
}

impl Weight {
    pub const ALL: [Self; 3] = [Self::Thin, Self::Medium, Self::Thick];

    pub const fn width(self) -> Px {
        match self {
            Self::Thin => STROKE_THIN,
            Self::Medium => STROKE_MEDIUM,
            Self::Thick => STROKE_THICK,
        }
    }

    pub const fn dot(self) -> Px {
        match self {
            Self::Thin => DOT_SMALL,
            Self::Medium => DOT_MEDIUM,
            Self::Thick => DOT_LARGE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Style {
    pub colour: Colour,
    pub weight: Weight,
}

impl Style {
    pub const fn new(colour: Colour, weight: Weight) -> Self {
        Self { colour, weight }
    }

    pub const fn width(self) -> Px {
        self.weight.width()
    }

    pub const fn rgba(self) -> Rgba {
        self.colour.rgba()
    }

    pub const fn highlight(self) -> Rgba {
        self.colour.highlight()
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            colour: Colour::Red,
            weight: Weight::Medium,
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ShapeId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: ShapeId,
    pub shape: Shape,
}

impl Annotation {
    pub const fn new(id: ShapeId, shape: Shape) -> Self {
        Self { id, shape }
    }

    pub fn with_shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Rect {
        rect: Rect,
        style: Style,
    },
    Ellipse {
        rect: Rect,
        style: Style,
    },
    Arrow {
        tail: Point,
        tip: Point,
        curve: Option<Point>,
        style: Style,
    },
    Pen {
        points: Vec<Point>,
        style: Style,
    },
    Highlight {
        points: Vec<Point>,
        style: Style,
    },
    Text {
        origin: Point,
        text: String,
        colour: Colour,
    },
    Step {
        center: Point,
        number: u32,
        colour: Colour,
    },
    Blur {
        rect: Rect,
    },
}

impl Shape {
    pub const fn rect(rect: Rect, style: Style) -> Self {
        Self::Rect { rect, style }
    }

    pub const fn ellipse(rect: Rect, style: Style) -> Self {
        Self::Ellipse { rect, style }
    }

    pub const fn arrow(tail: Point, tip: Point, style: Style) -> Self {
        Self::Arrow {
            tail,
            tip,
            curve: None,
            style,
        }
    }

    pub const fn pen(points: Vec<Point>, style: Style) -> Self {
        Self::Pen { points, style }
    }

    pub const fn highlight(points: Vec<Point>, style: Style) -> Self {
        Self::Highlight { points, style }
    }

    pub fn text(origin: Point, text: impl Into<String>, colour: Colour) -> Self {
        Self::Text {
            origin,
            text: text.into(),
            colour,
        }
    }

    pub const fn step(center: Point, number: u32, colour: Colour) -> Self {
        Self::Step {
            center,
            number,
            colour,
        }
    }

    pub const fn blur(rect: Rect) -> Self {
        Self::Blur { rect }
    }

    pub fn style(&self) -> Option<Style> {
        match self {
            Self::Rect { style, .. }
            | Self::Ellipse { style, .. }
            | Self::Arrow { style, .. }
            | Self::Pen { style, .. }
            | Self::Highlight { style, .. } => Some(*style),
            Self::Text { colour, .. } | Self::Step { colour, .. } => {
                Some(Style::new(*colour, Weight::Medium))
            }
            Self::Blur { .. } => None,
        }
    }

    pub fn bounds(&self) -> Rect {
        match self {
            Self::Rect { rect, .. } | Self::Ellipse { rect, .. } | Self::Blur { rect } => *rect,
            Self::Arrow {
                tail,
                tip,
                curve,
                style,
            } => {
                let mut bounds = Rect::from_points(*tail, *tip);
                if let Some(point) = curve {
                    bounds = bounds.union(Rect::from_points(*point, *point));
                }
                let width = style.width();
                let head = Px(width.0 + 6 + width.0 / 2);
                inflate(bounds, half(width).max(head))
            }
            Self::Pen { points, style } | Self::Highlight { points, style } => {
                points_bounds(points, half(style.width()))
            }
            Self::Text { origin, text, .. } => label_rect(*origin, text),
            Self::Step { center, .. } => step_rect(*center),
        }
    }

    pub fn translated(self, delta: Point) -> Self {
        let point = |point: Point| point.offset(delta.x, delta.y);
        match self {
            Self::Rect { rect, style } => Self::Rect {
                rect: rect.translated(delta),
                style,
            },
            Self::Ellipse { rect, style } => Self::Ellipse {
                rect: rect.translated(delta),
                style,
            },
            Self::Arrow {
                tail,
                tip,
                curve,
                style,
            } => Self::Arrow {
                tail: point(tail),
                tip: point(tip),
                curve: curve.map(point),
                style,
            },
            Self::Pen { points, style } => Self::Pen {
                points: points.into_iter().map(point).collect(),
                style,
            },
            Self::Highlight { points, style } => Self::Highlight {
                points: points.into_iter().map(point).collect(),
                style,
            },
            Self::Text {
                origin,
                text,
                colour,
            } => Self::Text {
                origin: point(origin),
                text,
                colour,
            },
            Self::Step {
                center,
                number,
                colour,
            } => Self::Step {
                center: point(center),
                number,
                colour,
            },
            Self::Blur { rect } => Self::Blur {
                rect: rect.translated(delta),
            },
        }
    }

    pub fn resized(self, from: Rect, to: Rect) -> Self {
        if from.is_empty() || to.is_empty() {
            return self;
        }
        let scale_point = |point: Point| scale_point(point, from, to);
        let scale_rect = |rect: Rect| scale_rect(rect, from, to);
        match self {
            Self::Rect { rect, style } => Self::Rect {
                rect: scale_rect(rect),
                style,
            },
            Self::Ellipse { rect, style } => Self::Ellipse {
                rect: scale_rect(rect),
                style,
            },
            Self::Arrow {
                tail,
                tip,
                curve,
                style,
            } => Self::Arrow {
                tail: scale_point(tail),
                tip: scale_point(tip),
                curve: curve.map(scale_point),
                style,
            },
            Self::Pen { points, style } => Self::Pen {
                points: points.into_iter().map(scale_point).collect(),
                style,
            },
            Self::Highlight { points, style } => Self::Highlight {
                points: points.into_iter().map(scale_point).collect(),
                style,
            },
            Self::Text {
                origin,
                text,
                colour,
            } => Self::Text {
                origin: scale_point(origin),
                text,
                colour,
            },
            Self::Step {
                center,
                number,
                colour,
            } => Self::Step {
                center: scale_point(center),
                number,
                colour,
            },
            Self::Blur { rect } => Self::Blur {
                rect: scale_rect(rect),
            },
        }
    }

    pub fn with_style(self, style: Style) -> Self {
        match self {
            Self::Rect { rect, .. } => Self::Rect { rect, style },
            Self::Ellipse { rect, .. } => Self::Ellipse { rect, style },
            Self::Arrow {
                tail, tip, curve, ..
            } => Self::Arrow {
                tail,
                tip,
                curve,
                style,
            },
            Self::Pen { points, .. } => Self::Pen { points, style },
            Self::Highlight { points, .. } => Self::Highlight { points, style },
            Self::Text { origin, text, .. } => Self::Text {
                origin,
                text,
                colour: style.colour,
            },
            Self::Step { center, number, .. } => Self::Step {
                center,
                number,
                colour: style.colour,
            },
            Self::Blur { rect } => Self::Blur { rect },
        }
    }

    pub fn with_text(self, value: impl Into<String>) -> Option<Self> {
        match self {
            Self::Text { origin, colour, .. } => Some(Self::Text {
                origin,
                text: value.into(),
                colour,
            }),
            _ => None,
        }
    }
}

pub const fn half(amount: Px) -> Px {
    Px(amount.0 / 2)
}

pub fn inflate(rect: Rect, amount: Px) -> Rect {
    Rect::from_edges(
        Px(rect.left().0 - amount.0),
        Px(rect.top().0 - amount.0),
        Px(rect.right().0 + amount.0),
        Px(rect.bottom().0 + amount.0),
    )
}

pub fn points_bounds(points: &[Point], padding: Px) -> Rect {
    let mut iter = points.iter();
    let Some(first) = iter.next() else {
        return Rect::ZERO;
    };
    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x;
    let mut bottom = first.y;
    for point in iter {
        left = left.min(point.x);
        top = top.min(point.y);
        right = right.max(point.x);
        bottom = bottom.max(point.y);
    }
    inflate(Rect::from_edges(left, top, right, bottom), padding)
}

pub const fn step_rect(center: Point) -> Rect {
    Rect::new(
        Px(center.x.0 - STEP_RADIUS.0),
        Px(center.y.0 - STEP_RADIUS.0),
        STEP_DIAMETER,
        STEP_DIAMETER,
    )
}

pub const fn text_scale(size: Px) -> i32 {
    if size.0 >= 12 {
        2
    } else {
        1
    }
}

pub const fn text_advance(size: Px) -> i32 {
    GLYPH_ADVANCE.0 * text_scale(size)
}

pub fn text_size(text: &str, size: Px) -> Size {
    let lines: Vec<_> = text.split('\n').collect();
    let count = lines
        .iter()
        .map(|line| line.trim_end_matches('\r').chars().count())
        .max()
        .unwrap_or(0) as i32;
    Size::new(
        Px(count * size.0.max(text_advance(size))),
        Px(lines.len() as i32 * (size.0 + 4)),
    )
}

pub fn label_rect(origin: Point, text: &str) -> Rect {
    let text = text_size(text, TEXT_SIZE);
    Rect::new(
        origin.x,
        origin.y,
        Px(text.width.0 + LABEL_PAD_X.0 * 2),
        Px(text.height.0 + LABEL_PAD_Y.0 * 2),
    )
}

pub const fn label_text_origin(origin: Point) -> Point {
    Point::new(
        Px(origin.x.0 + LABEL_PAD_X.0),
        Px(origin.y.0 + LABEL_PAD_Y.0),
    )
}

fn scale_x(value: Px, from: Rect, to: Rect) -> Px {
    if from.width.0 <= 0 {
        return to.x;
    }
    let offset = i64::from(value.0 - from.x.0);
    Px(to.x.0 + (offset * i64::from(to.width.0) / i64::from(from.width.0)) as i32)
}

fn scale_y(value: Px, from: Rect, to: Rect) -> Px {
    if from.height.0 <= 0 {
        return to.y;
    }
    let offset = i64::from(value.0 - from.y.0);
    Px(to.y.0 + (offset * i64::from(to.height.0) / i64::from(from.height.0)) as i32)
}

fn scale_point(point: Point, from: Rect, to: Rect) -> Point {
    Point::new(scale_x(point.x, from, to), scale_y(point.y, from, to))
}

fn scale_rect(rect: Rect, from: Rect, to: Rect) -> Rect {
    Rect::from_edges(
        scale_x(rect.left(), from, to),
        scale_y(rect.top(), from, to),
        scale_x(rect.right(), from, to),
        scale_y(rect.bottom(), from, to),
    )
}

pub fn smooth(points: &[Point]) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let last = points.len() - 1;
    let mut output = vec![points[0]];
    for index in 0..last {
        let p0 = points[index.saturating_sub(1)];
        let p1 = points[index];
        let p2 = points[index + 1];
        let p3 = points[(index + 2).min(last)];
        for step in 1..=8 {
            let t = f64::from(step) / 8.0;
            let point = catmull_rom(p0, p1, p2, p3, t);
            if output.last() != Some(&point) {
                output.push(point);
            }
        }
    }
    output
}

fn catmull_rom(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let x = catmull_rom_axis(p0.x, p1.x, p2.x, p3.x, t);
    let y = catmull_rom_axis(p0.y, p1.y, p2.y, p3.y, t);
    Point::new(Px(x), Px(y))
}

fn catmull_rom_axis(a: Px, b: Px, c: Px, d: Px, t: f64) -> i32 {
    let a = f64::from(a.0);
    let b = f64::from(b.0);
    let c = f64::from(c.0);
    let d = f64::from(d.0);
    let value = 0.5
        * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
            + (-a + 3.0 * b - 3.0 * c + d) * t * t * t);
    round(value)
}

pub fn round(value: f64) -> i32 {
    if value >= 0.0 {
        (value + 0.5).floor() as i32
    } else {
        (value - 0.5).ceil() as i32
    }
}
