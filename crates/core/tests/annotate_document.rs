use proptest::prelude::*;

use snapline_core::annotate::{Annotation, Colour, Document, Shape, ShapeId, Style, Weight};
use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::image::Image;

fn solid(width: u32, height: u32) -> Image {
    let mut image = match Image::bgra(width, height) {
        Ok(image) => image,
        Err(error) => panic!("{error}"),
    };
    for y in 0..height {
        for x in 0..width {
            let _ = image.set_pixel(x, y, [243, 243, 243, 255]);
        }
    }
    image
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
    Rect::new(Px(x), Px(y), Px(width), Px(height))
}

fn point(x: i32, y: i32) -> Point {
    Point::new(Px(x), Px(y))
}

fn steps(document: &Document) -> Vec<u32> {
    document
        .annotations()
        .iter()
        .filter_map(|annotation| match annotation.shape {
            Shape::Step { number, .. } => Some(number),
            _ => None,
        })
        .collect()
}

#[test]
fn step_numbers_follow_placement_order_and_renumber() {
    let mut document = Document::new(solid(32, 32));
    let first = document
        .add(Shape::step(point(4, 4), 1, Colour::Red))
        .expect("add");
    let second = document
        .add(Shape::step(point(12, 4), 1, Colour::Blue))
        .expect("add");
    let _third = document
        .add(Shape::step(point(20, 4), 1, Colour::Green))
        .expect("add");
    assert_eq!(steps(&document), vec![1, 2, 3]);
    assert_eq!(document.next_step(), 4);
    document.remove(second).expect("remove");
    assert_eq!(steps(&document), vec![1, 2]);
    document.undo().expect("undo");
    assert_eq!(steps(&document), vec![1, 2, 3]);
    document.undo().expect("undo");
    assert_eq!(steps(&document), vec![1, 2]);
    assert_eq!(document.annotation(first).is_some(), true);
}

#[test]
fn crop_shrinks_the_output_and_undoes() {
    let mut document = Document::new(solid(40, 30));
    document.crop_to(rect(5, 4, 20, 16)).expect("crop");
    let cropped = document.render().expect("render");
    assert_eq!(cropped.width(), 20);
    assert_eq!(cropped.height(), 16);
    assert_eq!(document.size().width, Px(20));
    document.undo().expect("undo");
    let full = document.render().expect("render");
    assert_eq!(full.width(), 40);
    assert_eq!(full.height(), 30);
}

#[test]
fn blur_uses_the_frozen_image_not_annotated_pixels() {
    let mut document = Document::new(solid(32, 32));
    document
        .add(Shape::highlight(
            vec![point(0, 16), point(31, 16)],
            Style::new(Colour::Red, Weight::Thick),
        ))
        .expect("add");
    document.add(Shape::blur(rect(12, 12, 8, 8))).expect("add");
    let image = document.render().expect("render");
    assert_eq!(image.pixel(16, 16), Ok([243, 243, 243, 255]));
}

#[test]
fn removing_a_command_clears_redo() {
    let mut document = Document::new(solid(16, 16));
    document
        .add(Shape::rect(rect(2, 2, 8, 8), Style::default()))
        .expect("add");
    document.undo().expect("undo");
    assert!(document.can_redo());
    document
        .add(Shape::ellipse(rect(4, 4, 6, 6), Style::default()))
        .expect("add");
    assert_eq!(document.can_redo(), false);
}

fn pick(ids: &[ShapeId], seed: i32) -> Option<ShapeId> {
    if ids.is_empty() {
        None
    } else {
        ids.get(seed as usize % ids.len()).copied()
    }
}

fn apply(document: &mut Document, ids: &mut Vec<ShapeId>, op: u8, a: i32, b: i32, c: i32) {
    match op % 14 {
        0 => {
            if let Ok(id) = document.add(Shape::rect(
                rect(a % 24 + 2, b % 24 + 2, c % 16 + 4, (a + b) % 16 + 4),
                Style::new(Colour::Red, Weight::Thin),
            )) {
                ids.push(id);
            }
        }
        1 => {
            if let Ok(id) = document.add(Shape::ellipse(
                rect(a % 20 + 2, b % 20 + 2, c % 16 + 4, (a + c) % 16 + 4),
                Style::new(Colour::Blue, Weight::Medium),
            )) {
                ids.push(id);
            }
        }
        2 => {
            let number = document.next_step();
            if let Ok(id) = document.add(Shape::step(
                point(a % 28 + 2, b % 28 + 2),
                number,
                Colour::Green,
            )) {
                ids.push(id);
            }
        }
        3 => {
            if let Ok(id) = document.add(Shape::text(
                point(a % 20 + 2, b % 20 + 2),
                format!("T{a}"),
                Colour::Orange,
            )) {
                ids.push(id);
            }
        }
        4 => {
            if let Some(id) = pick(ids, a) {
                let _ = document.move_by(id, point(c % 9 - 4, b % 9 - 4));
            }
        }
        5 => {
            if let Some(id) = pick(ids, b) {
                let _ = document.resize(id, rect(a % 24, b % 24, c % 18 + 4, (a + b) % 18 + 4));
            }
        }
        6 => {
            if let Some(id) = pick(ids, c) {
                let _ = document.restyle(
                    id,
                    Style::new(Colour::ALL[(a as usize) % 6], Weight::ALL[(b as usize) % 3]),
                );
            }
        }
        7 => {
            if let Some(id) = pick(ids, a) {
                let _ = document.edit_text(id, format!("E{b}"));
            }
        }
        8 => {
            if let Some(id) = pick(ids, b) {
                let _ = document.remove(id);
            }
        }
        9 => {
            let crop = rect(a % 20 + 2, b % 20 + 2, c % 12 + 4, (a + b) % 12 + 4);
            let _ = document.crop_to(crop);
        }
        10 => {
            if let Ok(id) = document.add(Shape::pen(
                vec![
                    point(a % 28 + 2, b % 28 + 2),
                    point(c % 28 + 2, (a + b) % 28 + 2),
                    point((b + c) % 28 + 2, (a + c) % 28 + 2),
                ],
                Style::new(Colour::Green, Weight::Thick),
            )) {
                ids.push(id);
            }
        }
        11 => {
            if let Ok(id) = document.add(Shape::highlight(
                vec![
                    point(a % 28 + 2, b % 28 + 2),
                    point(c % 28 + 2, (b + c) % 28 + 2),
                ],
                Style::new(Colour::Yellow, Weight::Medium),
            )) {
                ids.push(id);
            }
        }
        12 => {
            if let Ok(id) = document.add(Shape::blur(rect(
                a % 20 + 2,
                b % 20 + 2,
                c % 12 + 4,
                (b + c) % 12 + 4,
            ))) {
                ids.push(id);
            }
        }
        _ => {
            if let Ok(id) = document.add(Shape::arrow(
                point(a % 28 + 2, b % 28 + 2),
                point(c % 28 + 2, (a + c) % 28 + 2),
                Style::new(Colour::Red, Weight::Medium),
            )) {
                ids.push(id);
            }
        }
    }
}

proptest! {
    #[test]
    fn undo_after_any_edits_returns_the_start_state(
        ops in proptest::collection::vec(any::<u8>(), 0..96),
    ) {
        let mut document = Document::new(solid(32, 32));
        let start: Vec<Annotation> = document.annotations().to_vec();
        let start_crop = document.crop();
        let mut ids: Vec<ShapeId> = Vec::new();
        for chunk in ops.chunks(4) {
            let op = chunk[0];
            let a = i32::from(*chunk.get(1).unwrap_or(&0));
            let b = i32::from(*chunk.get(2).unwrap_or(&0));
            let c = i32::from(*chunk.get(3).unwrap_or(&0));
            apply(&mut document, &mut ids, op, a, b, c);
        }
        let end: Vec<Annotation> = document.annotations().to_vec();
        let end_crop = document.crop();
        while document.can_undo() {
            prop_assert!(document.undo().is_ok());
        }
        prop_assert_eq!(document.annotations(), start.as_slice());
        prop_assert_eq!(document.crop(), start_crop);
        while document.can_redo() {
            prop_assert!(document.redo().is_ok());
        }
        prop_assert_eq!(document.annotations(), end.as_slice());
        prop_assert_eq!(document.crop(), end_crop);
        while document.can_undo() {
            prop_assert!(document.undo().is_ok());
        }
        prop_assert_eq!(document.annotations(), start.as_slice());
        prop_assert_eq!(document.crop(), start_crop);
    }
}
