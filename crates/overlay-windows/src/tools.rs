use std::time::{Duration, Instant};

use snapline_core::annotate::{
    label_rect, text_size, Annotation, Colour, Command, Document, Shape, ShapeId, Style, TEXT_SIZE,
};
use snapline_core::geometry::{Grip, Point, Px, Rect};
use snapline_core::image::Image;
use snapline_core::overlay::{Rgb, Tool};

use windows::Win32::Foundation::{GlobalFree, HANDLE};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::{AnnotationColour, StrokeSize};

pub(crate) const PILL_DURATION_MS: u32 = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Press {
    Handled,
    Text(Point),
    Copied(Rgb),
}

#[derive(Debug, Clone)]
pub(crate) struct ColourPill {
    text: String,
    shown: Instant,
}

impl ColourPill {
    pub(crate) fn new(rgb: Rgb, now: Instant) -> Self {
        Self {
            text: pill_text(rgb),
            shown: now,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn visible(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.shown)
            < Duration::from_millis(u64::from(PILL_DURATION_MS))
    }
}

#[derive(Debug, Clone, Copy)]
enum DragKind {
    Create(Tool),
    Move(ShapeId),
    Resize(ShapeId, Grip),
    Curve(ShapeId),
}

#[derive(Debug, Clone)]
struct Drag {
    kind: DragKind,
    start: Point,
    last: Point,
    current: Point,
    points: Vec<Point>,
    original: Option<Annotation>,
    style: Style,
    steps: usize,
    square: bool,
}

#[derive(Debug, Clone)]
struct CropDrag {
    before: Rect,
    start: Point,
    grip: Option<Grip>,
    current: Rect,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Session {
    selected: Option<ShapeId>,
    drag: Option<Drag>,
    crop: Option<CropDrag>,
}

impl Session {
    pub(crate) fn selected(&self) -> Option<ShapeId> {
        self.selected
    }

    pub(crate) fn is_active(&self) -> bool {
        self.drag.is_some() || self.crop.is_some()
    }

    pub(crate) fn reset(&mut self) {
        self.selected = None;
        self.drag = None;
        self.crop = None;
    }

    pub(crate) fn sanitize(&mut self, document: &Document) {
        if self
            .selected
            .is_some_and(|id| document.annotation(id).is_none())
        {
            self.selected = None;
        }
    }

    pub(crate) fn press(
        &mut self,
        tool: Tool,
        point: Point,
        document: &mut Document,
        style: Style,
    ) -> Press {
        self.crop = None;
        self.drag = None;
        match tool {
            Tool::Move => {
                self.press_move(point, document, style);
                Press::Handled
            }
            Tool::Text => Press::Text(point),
            Tool::Step => {
                let number = document.next_step();
                if let Ok(id) = document.add(Shape::step(point, number, style.colour)) {
                    self.selected = Some(id);
                }
                Press::Handled
            }
            Tool::Colour => match sample(document, point) {
                Some(rgb) => Press::Copied(rgb),
                None => Press::Handled,
            },
            Tool::Crop => {
                self.press_crop(point, document);
                Press::Handled
            }
            Tool::Arrow => {
                if let Some(id) = arrow_at(document, point) {
                    if let Some(original) = document.annotation(id).cloned() {
                        self.selected = Some(id);
                        self.drag = Some(Drag {
                            kind: DragKind::Curve(id),
                            start: point,
                            last: point,
                            current: point,
                            points: Vec::new(),
                            original: Some(original),
                            style,
                            steps: 0,
                            square: false,
                        });
                        return Press::Handled;
                    }
                }
                self.start_create(tool, point, style);
                Press::Handled
            }
            Tool::Rect | Tool::Ellipse | Tool::Pen | Tool::Highlighter | Tool::Blur => {
                self.start_create(tool, point, style);
                Press::Handled
            }
        }
    }

    pub(crate) fn move_to(&mut self, point: Point, document: &mut Document, shift: bool) {
        if let Some(crop) = &mut self.crop {
            crop.current = crop_rect(crop, point, document.full_rect());
            return;
        }
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.current = point;
        drag.square = shift;
        match drag.kind {
            DragKind::Create(tool) => {
                if matches!(tool, Tool::Pen | Tool::Highlighter)
                    && drag.points.last() != Some(&point)
                {
                    drag.points.push(point);
                }
            }
            DragKind::Move(id) => {
                let delta =
                    Point::new(Px(point.x.0 - drag.last.x.0), Px(point.y.0 - drag.last.y.0));
                if delta.x.0 == 0 && delta.y.0 == 0 {
                    return;
                }
                if document.move_by(id, delta).is_ok() {
                    drag.steps += 1;
                    drag.last = point;
                }
            }
            DragKind::Resize(id, grip) => {
                let Some(original) = &drag.original else {
                    return;
                };
                let target = grip.resize_to(original.shape.bounds(), point);
                if document.resize(id, target).is_ok() {
                    drag.steps += 1;
                }
            }
            DragKind::Curve(id) => {
                let Some(original) = &drag.original else {
                    return;
                };
                let Some(shape) = with_curve(&original.shape, point) else {
                    return;
                };
                let Some(current) = document.annotation(id).cloned() else {
                    return;
                };
                let after = original.clone().with_shape(shape);
                let command = Command::Move {
                    before: current,
                    after,
                };
                if document.execute(command).is_ok() {
                    drag.steps += 1;
                }
            }
        }
    }

    pub(crate) fn release(&mut self, point: Point, document: &mut Document, shift: bool) {
        if let Some(crop) = self.crop.take() {
            let target = crop_rect(&crop, point, document.full_rect());
            if !target.is_empty() {
                let _ = document.crop_to(target);
            }
            return;
        }
        let Some(mut drag) = self.drag.take() else {
            return;
        };
        drag.current = point;
        drag.square = shift;
        if matches!(drag.kind, DragKind::Create(Tool::Pen | Tool::Highlighter))
            && drag.points.last() != Some(&point)
        {
            drag.points.push(point);
        }
        if drag.steps > 0 {
            for _ in 0..drag.steps {
                let _ = document.undo();
            }
        }
        match drag.kind {
            DragKind::Create(tool) => {
                if let Some(shape) = create_shape(tool, &drag) {
                    if let Ok(id) = document.add(shape) {
                        self.selected = Some(id);
                    }
                }
            }
            DragKind::Move(_) => {
                if drag.steps == 0 {
                    return;
                }
                let Some(before) = drag.original else {
                    return;
                };
                let delta = Point::new(
                    Px(drag.current.x.0 - drag.start.x.0),
                    Px(drag.current.y.0 - drag.start.y.0),
                );
                let after = before
                    .clone()
                    .with_shape(before.shape.clone().translated(delta));
                let _ = document.execute(Command::Move { before, after });
            }
            DragKind::Resize(id, grip) => {
                let _ = id;
                if drag.steps == 0 {
                    return;
                }
                let Some(before) = drag.original else {
                    return;
                };
                let from = before.shape.bounds();
                let target = grip.resize_to(from, drag.current);
                if target.is_empty() {
                    return;
                }
                let after = before
                    .clone()
                    .with_shape(before.shape.clone().resized(from, target));
                let _ = document.execute(Command::Resize { before, after });
            }
            DragKind::Curve(id) => {
                let _ = id;
                if drag.steps == 0 {
                    return;
                }
                let Some(before) = drag.original else {
                    return;
                };
                let Some(shape) = with_curve(&before.shape, drag.current) else {
                    return;
                };
                let after = before.clone().with_shape(shape);
                let _ = document.execute(Command::Move { before, after });
            }
        }
    }

    pub(crate) fn cancel(&mut self, document: &mut Document) -> bool {
        let active = self.is_active();
        if let Some(drag) = self.drag.take() {
            for _ in 0..drag.steps {
                let _ = document.undo();
            }
        }
        self.crop = None;
        active
    }

    pub(crate) fn delete(&mut self, document: &mut Document) -> bool {
        let Some(id) = self.selected.take() else {
            return false;
        };
        document.remove(id).is_ok()
    }

    pub(crate) fn preview(&self) -> Option<Shape> {
        let drag = self.drag.as_ref()?;
        match drag.kind {
            DragKind::Create(tool) => create_shape(tool, drag),
            _ => None,
        }
    }

    pub(crate) fn crop_preview(&self) -> Option<Rect> {
        self.crop.as_ref().map(|crop| crop.current)
    }

    fn press_move(&mut self, point: Point, document: &mut Document, style: Style) {
        if let Some((selected, grip)) = handle_at(document, self.selected, point) {
            if let Some(annotation) = document.annotation(selected) {
                self.selected = Some(selected);
                self.start_drag(
                    DragKind::Resize(selected, grip),
                    point,
                    Some(annotation.clone()),
                    style,
                );
                return;
            }
        }
        let hit = hit_shape(document, point);
        self.selected = hit;
        if let Some(id) = hit {
            if let Some(annotation) = document.annotation(id) {
                self.start_drag(DragKind::Move(id), point, Some(annotation.clone()), style);
            }
        }
    }

    fn press_crop(&mut self, point: Point, document: &Document) {
        let before = document.crop();
        let grip = before.grip_at(point);
        let current = match grip {
            Some(grip) => grip.resize_to(before, point),
            None => Rect::from_points(point, point),
        };
        self.crop = Some(CropDrag {
            before,
            start: point,
            grip,
            current,
        });
    }

    fn start_create(&mut self, tool: Tool, point: Point, style: Style) {
        self.selected = None;
        self.drag = Some(Drag {
            kind: DragKind::Create(tool),
            start: point,
            last: point,
            current: point,
            points: vec![point],
            original: None,
            style,
            steps: 0,
            square: false,
        });
    }

    fn start_drag(
        &mut self,
        kind: DragKind,
        point: Point,
        original: Option<Annotation>,
        style: Style,
    ) {
        self.drag = Some(Drag {
            kind,
            start: point,
            last: point,
            current: point,
            points: Vec::new(),
            original,
            style,
            steps: 0,
            square: false,
        });
    }
}

pub(crate) fn style(colour: AnnotationColour, stroke: StrokeSize) -> Style {
    Style::new(self::colour(colour), weight(stroke))
}

pub(crate) fn colour(value: AnnotationColour) -> Colour {
    match value {
        AnnotationColour::Red => Colour::Red,
        AnnotationColour::Orange => Colour::Orange,
        AnnotationColour::Yellow => Colour::Yellow,
        AnnotationColour::Green => Colour::Green,
        AnnotationColour::Blue => Colour::Blue,
        AnnotationColour::White => Colour::White,
    }
}

pub(crate) fn weight(value: StrokeSize) -> snapline_core::annotate::Weight {
    match value {
        StrokeSize::Thin => snapline_core::annotate::Weight::Thin,
        StrokeSize::Medium => snapline_core::annotate::Weight::Medium,
        StrokeSize::Thick => snapline_core::annotate::Weight::Thick,
    }
}

pub(crate) fn add_text(
    document: &mut Document,
    origin: Point,
    text: impl Into<String>,
    colour: Colour,
) -> Option<ShapeId> {
    let text = text.into();
    if text.trim().is_empty() {
        return None;
    }
    document.add(Shape::text(origin, text, colour)).ok()
}

pub(crate) fn text_box(origin: Point, scale: f32) -> Rect {
    let label = label_rect(origin, "");
    let size = text_size("", TEXT_SIZE);
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    Rect::new(
        origin.x,
        origin.y,
        Px((120.0 * scale).round() as i32),
        Px(((size.height.0 + 8).max(22) as f32 * scale).round() as i32),
    )
    .union(label)
}

pub(crate) fn sample(document: &Document, point: Point) -> Option<Rgb> {
    sample_image(document.image(), point)
}

pub(crate) fn sample_image(image: &Image, point: Point) -> Option<Rgb> {
    let x = u32::try_from(point.x.0).ok()?;
    let y = u32::try_from(point.y.0).ok()?;
    let pixel = image.pixel(x, y).ok()?;
    Some(Rgb::new(pixel[2], pixel[1], pixel[0]))
}

pub(crate) fn hex(rgb: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b)
}

pub(crate) fn pill_text(rgb: Rgb) -> String {
    format!("Copied {}", hex(rgb))
}

pub(crate) fn copy_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    // SAFETY: allocates clipboard memory and copies the null terminated UTF-16 text into it.
    unsafe {
        let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
            return false;
        };
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            let _ = GlobalFree(Some(handle));
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr().cast::<u8>(), pointer.cast::<u8>(), bytes);
        let _ = GlobalUnlock(handle);
        if OpenClipboard(None).is_err() {
            let _ = GlobalFree(Some(handle));
            return false;
        }
        let _ = EmptyClipboard();
        let mut owned = true;
        if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(handle.0))).is_ok() {
            owned = false;
        }
        let _ = CloseClipboard();
        if owned {
            let _ = GlobalFree(Some(handle));
        }
        true
    }
}

fn create_shape(tool: Tool, drag: &Drag) -> Option<Shape> {
    let rect = drag_rect(drag.start, drag.current, drag.square);
    match tool {
        Tool::Rect => {
            if rect.is_empty() {
                None
            } else {
                Some(Shape::rect(rect, drag.style))
            }
        }
        Tool::Ellipse => {
            if rect.is_empty() {
                None
            } else {
                Some(Shape::ellipse(rect, drag.style))
            }
        }
        Tool::Blur => {
            if rect.is_empty() {
                None
            } else {
                Some(Shape::blur(rect))
            }
        }
        Tool::Arrow => {
            if drag.start == drag.current {
                None
            } else {
                Some(Shape::arrow(drag.start, drag.current, drag.style))
            }
        }
        Tool::Pen => {
            if drag.points.is_empty() {
                None
            } else {
                Some(Shape::pen(drag.points.clone(), drag.style))
            }
        }
        Tool::Highlighter => {
            if drag.points.is_empty() {
                None
            } else {
                Some(Shape::highlight(drag.points.clone(), drag.style))
            }
        }
        Tool::Move | Tool::Text | Tool::Step | Tool::Crop | Tool::Colour => None,
    }
}

fn drag_rect(start: Point, current: Point, square: bool) -> Rect {
    if !square {
        return Rect::from_points(start, current);
    }
    let dx = current.x.0 - start.x.0;
    let dy = current.y.0 - start.y.0;
    let size = dx.abs().max(dy.abs());
    let end = Point::new(
        Px(start.x.0 + if dx < 0 { -size } else { size }),
        Px(start.y.0 + if dy < 0 { -size } else { size }),
    );
    Rect::from_points(start, end)
}

fn crop_rect(crop: &CropDrag, point: Point, full: Rect) -> Rect {
    let rect = match crop.grip {
        Some(grip) => grip.resize_to(crop.before, point),
        None => Rect::from_points(crop.start, point),
    };
    rect.intersection(full).unwrap_or(Rect::ZERO)
}

fn handle_at(
    document: &Document,
    selected: Option<ShapeId>,
    point: Point,
) -> Option<(ShapeId, Grip)> {
    if let Some(id) = selected {
        if let Some(annotation) = document.annotation(id) {
            if let Some(grip) = annotation.shape.bounds().grip_at(point) {
                return Some((id, grip));
            }
        }
    }
    document.annotations().iter().rev().find_map(|annotation| {
        annotation
            .shape
            .bounds()
            .grip_at(point)
            .map(|grip| (annotation.id, grip))
    })
}

fn hit_shape(document: &Document, point: Point) -> Option<ShapeId> {
    document
        .annotations()
        .iter()
        .rev()
        .find(|annotation| annotation.shape.bounds().contains(point))
        .map(|annotation| annotation.id)
}

fn arrow_at(document: &Document, point: Point) -> Option<ShapeId> {
    document.annotations().iter().rev().find_map(|annotation| {
        let Shape::Arrow {
            tail, tip, style, ..
        } = &annotation.shape
        else {
            return None;
        };
        let middle = Point::new(Px((tail.x.0 + tip.x.0) / 2), Px((tail.y.0 + tip.y.0) / 2));
        let reach = (style.width().0 * 2).max(10);
        if (point.x.0 - middle.x.0).abs() <= reach && (point.y.0 - middle.y.0).abs() <= reach {
            Some(annotation.id)
        } else {
            None
        }
    })
}

fn with_curve(shape: &Shape, point: Point) -> Option<Shape> {
    match shape {
        Shape::Arrow {
            tail, tip, style, ..
        } => Some(Shape::Arrow {
            tail: *tail,
            tip: *tip,
            curve: Some(point),
            style: *style,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use snapline_core::annotate::Weight;
    use snapline_core::image::Image;

    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    fn blank(width: u32, height: u32) -> Document {
        let Ok(image) = Image::bgra(width, height) else {
            panic!()
        };
        Document::new(image)
    }

    fn red_style() -> Style {
        Style::new(Colour::Red, Weight::Medium)
    }

    fn drag(session: &mut Session, document: &mut Document, tool: Tool, from: Point, to: Point) {
        let _ = session.press(tool, from, document, red_style());
        session.move_to(to, document, false);
        session.release(to, document, false);
    }

    #[test]
    fn rectangle_and_ellipse_use_the_drag_bounds() {
        let mut document = blank(300, 200);
        let mut session = Session::default();
        drag(
            &mut session,
            &mut document,
            Tool::Rect,
            point(10, 10),
            point(110, 70),
        );
        drag(
            &mut session,
            &mut document,
            Tool::Ellipse,
            point(20, 20),
            point(80, 100),
        );
        assert_eq!(document.annotations().len(), 2);
        assert_eq!(
            document.annotations()[0].shape,
            Shape::rect(rect(10, 10, 100, 60), red_style())
        );
        assert_eq!(
            document.annotations()[1].shape,
            Shape::ellipse(rect(20, 20, 60, 80), red_style())
        );
    }

    #[test]
    fn shift_makes_a_square_and_a_circle() {
        let mut document = blank(300, 200);
        let mut session = Session::default();
        let _ = session.press(Tool::Rect, point(10, 10), &mut document, red_style());
        session.move_to(point(110, 50), &mut document, true);
        session.release(point(110, 50), &mut document, true);
        assert_eq!(
            document.annotations()[0].shape.bounds(),
            rect(10, 10, 100, 100)
        );
        let _ = session.press(Tool::Ellipse, point(20, 20), &mut document, red_style());
        session.move_to(point(60, 90), &mut document, true);
        session.release(point(60, 90), &mut document, true);
        assert_eq!(
            document.annotations()[1].shape.bounds(),
            rect(20, 20, 70, 70)
        );
    }

    #[test]
    fn arrow_pen_highlighter_and_blur_are_created() {
        let mut document = blank(300, 200);
        let mut session = Session::default();
        drag(
            &mut session,
            &mut document,
            Tool::Arrow,
            point(10, 20),
            point(90, 60),
        );
        drag(
            &mut session,
            &mut document,
            Tool::Pen,
            point(10, 10),
            point(40, 40),
        );
        drag(
            &mut session,
            &mut document,
            Tool::Highlighter,
            point(5, 5),
            point(50, 15),
        );
        drag(
            &mut session,
            &mut document,
            Tool::Blur,
            point(100, 100),
            point(160, 140),
        );
        assert_eq!(document.annotations().len(), 4);
        assert_eq!(
            document.annotations()[0].shape,
            Shape::arrow(point(10, 20), point(90, 60), red_style())
        );
        assert_eq!(
            document.annotations()[1].shape,
            Shape::pen(vec![point(10, 10), point(40, 40)], red_style())
        );
        assert_eq!(
            document.annotations()[2].shape,
            Shape::highlight(vec![point(5, 5), point(50, 15)], red_style())
        );
        assert_eq!(
            document.annotations()[3].shape,
            Shape::blur(rect(100, 100, 60, 40))
        );
    }

    #[test]
    fn arrow_middle_drag_sets_a_curve() {
        let mut document = blank(300, 200);
        let style = red_style();
        let id = document
            .add(Shape::arrow(point(10, 100), point(210, 100), style))
            .unwrap();
        let mut session = Session::default();
        assert_eq!(
            session.press(Tool::Arrow, point(110, 100), &mut document, style),
            Press::Handled
        );
        session.move_to(point(110, 40), &mut document, false);
        session.release(point(110, 40), &mut document, false);
        let Shape::Arrow { curve, .. } = &document.annotation(id).unwrap().shape else {
            panic!()
        };
        assert_eq!(*curve, Some(point(110, 40)));
        let _ = document.undo();
        let Shape::Arrow { curve, .. } = &document.annotation(id).unwrap().shape else {
            panic!()
        };
        assert_eq!(*curve, None);
    }

    #[test]
    fn moving_a_selected_shape_is_one_undoable_command() {
        let mut document = blank(400, 300);
        let style = red_style();
        let id = document
            .add(Shape::rect(rect(50, 50, 100, 80), style))
            .unwrap();
        let mut session = Session::default();
        assert_eq!(
            session.press(Tool::Move, point(100, 90), &mut document, style),
            Press::Handled
        );
        assert_eq!(session.selected(), Some(id));
        session.move_to(point(120, 100), &mut document, false);
        session.move_to(point(140, 120), &mut document, false);
        session.release(point(140, 120), &mut document, false);
        assert_eq!(
            document.annotation(id).unwrap().shape.bounds(),
            rect(90, 80, 100, 80)
        );
        assert_eq!(document.undo_len(), 2);
        let _ = document.undo();
        assert_eq!(
            document.annotation(id).unwrap().shape.bounds(),
            rect(50, 50, 100, 80)
        );
    }

    #[test]
    fn resizing_from_a_handle_is_one_undoable_command() {
        let mut document = blank(400, 300);
        let style = red_style();
        let id = document
            .add(Shape::rect(rect(50, 50, 100, 80), style))
            .unwrap();
        let mut session = Session::default();
        let handle = Grip::SouthEast.point(rect(50, 50, 100, 80));
        let _ = session.press(Tool::Move, handle, &mut document, style);
        session.move_to(point(190, 170), &mut document, false);
        session.release(point(190, 170), &mut document, false);
        assert_eq!(
            document.annotation(id).unwrap().shape.bounds(),
            rect(50, 50, 140, 120)
        );
        assert_eq!(document.undo_len(), 2);
    }

    #[test]
    fn clicking_empty_space_deselects_and_delete_removes() {
        let mut document = blank(400, 300);
        let style = red_style();
        let id = document
            .add(Shape::rect(rect(50, 50, 100, 80), style))
            .unwrap();
        let mut session = Session::default();
        let _ = session.press(Tool::Move, point(60, 60), &mut document, style);
        assert_eq!(session.selected(), Some(id));
        let _ = session.press(Tool::Move, point(300, 200), &mut document, style);
        assert_eq!(session.selected(), None);
        let _ = session.press(Tool::Move, point(60, 60), &mut document, style);
        assert!(session.delete(&mut document));
        assert!(document.is_empty());
    }

    #[test]
    fn steps_count_up_and_renumber_after_removal() {
        let mut document = blank(400, 300);
        let style = red_style();
        let mut session = Session::default();
        for (x, y) in [(20, 20), (60, 20), (100, 20)] {
            assert_eq!(
                session.press(Tool::Step, point(x, y), &mut document, style),
                Press::Handled
            );
        }
        let numbers: Vec<u32> = document
            .annotations()
            .iter()
            .filter_map(|annotation| match annotation.shape {
                Shape::Step { number, .. } => Some(number),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(document.next_step(), 4);
        let ids: Vec<ShapeId> = document.annotations().iter().map(|item| item.id).collect();
        document.remove(ids[1]).unwrap();
        let numbers: Vec<u32> = document
            .annotations()
            .iter()
            .filter_map(|annotation| match annotation.shape {
                Shape::Step { number, .. } => Some(number),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, vec![1, 2]);
        assert_eq!(document.next_step(), 3);
    }

    #[test]
    fn crop_drag_is_undoable() {
        let mut document = blank(400, 300);
        let mut session = Session::default();
        let _ = session.press(Tool::Crop, point(400, 300), &mut document, red_style());
        session.move_to(point(200, 150), &mut document, false);
        session.release(point(200, 150), &mut document, false);
        assert_eq!(document.crop(), rect(0, 0, 200, 150));
        let _ = document.undo();
        assert_eq!(document.crop(), rect(0, 0, 400, 300));
    }

    #[test]
    fn crop_drag_from_empty_space_uses_a_new_rectangle() {
        let mut document = blank(400, 300);
        let mut session = Session::default();
        let _ = session.press(Tool::Crop, point(20, 20), &mut document, red_style());
        session.move_to(point(120, 90), &mut document, false);
        session.release(point(120, 90), &mut document, false);
        assert_eq!(document.crop(), rect(20, 20, 100, 70));
    }

    #[test]
    fn colour_picker_samples_the_frozen_pixel_and_formats_the_pill() {
        let base = blank(20, 20);
        let mut image = base.image().clone();
        image.set_pixel(5, 6, [0x4d, 0x48, 0xe5, 255]).unwrap();
        let document = Document::new(image);
        let mut target = document.clone();
        let mut session = Session::default();
        assert_eq!(
            session.press(Tool::Colour, point(5, 6), &mut target, red_style()),
            Press::Copied(Rgb::new(0xe5, 0x48, 0x4d))
        );
        assert_eq!(hex(Rgb::new(0xe5, 0x48, 0x4d)), "#E5484D");
        let pill = ColourPill::new(Rgb::new(0xe5, 0x48, 0x4d), Instant::now());
        assert_eq!(pill.text(), "Copied #E5484D");
    }

    #[test]
    fn colour_pill_lasts_one_and_a_half_seconds() {
        let now = Instant::now();
        let pill = ColourPill::new(Rgb::new(0x30, 0xa4, 0x6c), now);
        assert!(pill.visible(now));
        assert!(pill.visible(now + Duration::from_millis(1499)));
        assert!(!pill.visible(now + Duration::from_millis(1500)));
    }

    #[test]
    fn toolbar_state_follows_the_document_history() {
        let mut document = blank(40, 30);
        assert!(!document.can_undo());
        assert!(!document.can_redo());
        document
            .add(Shape::rect(rect(1, 1, 10, 10), red_style()))
            .unwrap();
        assert!(document.can_undo());
        assert!(!document.can_redo());
        let _ = document.undo();
        assert!(!document.can_undo());
        assert!(document.can_redo());
        let _ = document.redo();
        assert!(document.can_undo());
        assert!(!document.can_redo());
    }

    #[test]
    fn text_box_is_at_least_the_label_size() {
        let area = text_box(point(10, 20), 2.0);
        assert_eq!(area.x, Px(10));
        assert_eq!(area.y, Px(20));
        assert!(area.width >= Px(120));
        assert!(area.width >= label_rect(point(0, 0), "").width);
    }

    #[test]
    #[ignore = "desktop"]
    fn copies_text_to_the_clipboard() {
        assert!(copy_text("#E5484D"));
    }
}
