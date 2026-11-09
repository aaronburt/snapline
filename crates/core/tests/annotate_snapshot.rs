use snapline_core::annotate::{
    fingerprint, Colour, Document, Raster, Rgba, Shape, Style, Tolerance, Weight,
};
use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::image::Image;

const BACKGROUND: [u8; 4] = [243, 243, 243, 255];

fn solid(width: u32, height: u32) -> Image {
    let mut image = match Image::bgra(width, height) {
        Ok(image) => image,
        Err(error) => panic!("{error}"),
    };
    for y in 0..height {
        for x in 0..width {
            let _ = image.set_pixel(x, y, BACKGROUND);
        }
    }
    image
}

fn pattern(width: u32, height: u32) -> Image {
    let mut image = match Image::bgra(width, height) {
        Ok(image) => image,
        Err(error) => panic!("{error}"),
    };
    for y in 0..height {
        for x in 0..width {
            let pixel = [
                ((x * 255) / width.max(1)) as u8,
                ((y * 255) / height.max(1)) as u8,
                128,
                255,
            ];
            let _ = image.set_pixel(x, y, pixel);
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

fn render(shape: &Shape) -> Image {
    let frozen = solid(64, 64);
    render_over(shape, &frozen)
}

fn render_over(shape: &Shape, frozen: &Image) -> Image {
    let mut raster = Raster::from_image(frozen.clone());
    raster.draw(shape, frozen);
    raster.into_image()
}

fn classify(pixel: [u8; 4]) -> char {
    if pixel[3] < 8 {
        return '.';
    }
    let colour = [
        i32::from(pixel[2]),
        i32::from(pixel[1]),
        i32::from(pixel[0]),
    ];
    let palette: [(char, [i32; 3]); 16] = [
        ('W', [255, 255, 255]),
        ('-', [243, 243, 243]),
        ('k', [20, 20, 20]),
        ('+', [128, 128, 128]),
        ('R', [229, 72, 77]),
        ('O', [247, 107, 21]),
        ('Y', [255, 214, 10]),
        ('G', [48, 164, 108]),
        ('B', [0, 144, 255]),
        ('r', [234, 175, 178]),
        ('o', [240, 186, 158]),
        ('y', [250, 227, 113]),
        ('g', [182, 215, 199]),
        ('b', [194, 213, 248]),
        ('w', [232, 232, 232]),
        ('K', [60, 60, 60]),
    ];
    let mut best = '.';
    let mut best_distance = i64::MAX;
    for (glyph, reference) in palette {
        let mut distance = 0_i64;
        for channel in 0..3 {
            let delta = i64::from(colour[channel] - reference[channel]);
            distance += delta * delta;
        }
        if distance < best_distance {
            best_distance = distance;
            best = glyph;
        }
    }
    best
}

fn art(image: &Image) -> String {
    let mut output = format!("{}x{} +2\n", image.width(), image.height());
    for y in (0..image.height()).step_by(2) {
        for x in (0..image.width()).step_by(2) {
            let pixel = image.pixel(x, y).unwrap_or([0; 4]);
            output.push(classify(pixel));
        }
        output.push('\n');
    }
    output.push_str(&format!("fnv64 {:016x}\n", fingerprint(image)));
    output
}

#[test]
fn tolerance_two_accepts_rounding() {
    assert!(Tolerance::TWO.within([1, 2, 3, 255], [3, 4, 5, 253]));
    assert!(!Tolerance::TWO.within([1, 2, 3, 255], [4, 2, 3, 255]));
}

#[test]
fn png_starts_with_the_png_signature() {
    let png = Raster::from_image(solid(4, 4)).to_png();
    assert_eq!(&png[..8], &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
}

#[test]
fn rect_thin_red() {
    let image = render(&Shape::rect(
        rect(12, 12, 40, 32),
        Style::new(Colour::Red, Weight::Thin),
    ));
    insta::assert_snapshot!("rect_thin_red", art(&image));
}

#[test]
fn rect_medium_blue() {
    let image = render(&Shape::rect(
        rect(12, 12, 40, 32),
        Style::new(Colour::Blue, Weight::Medium),
    ));
    insta::assert_snapshot!("rect_medium_blue", art(&image));
}

#[test]
fn rect_thick_green() {
    let image = render(&Shape::rect(
        rect(12, 12, 40, 32),
        Style::new(Colour::Green, Weight::Thick),
    ));
    insta::assert_snapshot!("rect_thick_green", art(&image));
}

#[test]
fn ellipse_thin_orange() {
    let image = render(&Shape::ellipse(
        rect(10, 14, 44, 36),
        Style::new(Colour::Orange, Weight::Thin),
    ));
    insta::assert_snapshot!("ellipse_thin_orange", art(&image));
}

#[test]
fn ellipse_medium_red() {
    let image = render(&Shape::ellipse(
        rect(10, 14, 44, 36),
        Style::new(Colour::Red, Weight::Medium),
    ));
    insta::assert_snapshot!("ellipse_medium_red", art(&image));
}

#[test]
fn ellipse_thick_white() {
    let image = render(&Shape::ellipse(
        rect(10, 14, 44, 36),
        Style::new(Colour::White, Weight::Thick),
    ));
    insta::assert_snapshot!("ellipse_thick_white", art(&image));
}

#[test]
fn arrow_straight_red() {
    let image = render(&Shape::arrow(
        point(10, 52),
        point(54, 12),
        Style::new(Colour::Red, Weight::Medium),
    ));
    insta::assert_snapshot!("arrow_straight_red", art(&image));
}

#[test]
fn arrow_curved_blue() {
    let image = render(&Shape::Arrow {
        tail: point(10, 52),
        tip: point(54, 14),
        curve: Some(point(26, 2)),
        style: Style::new(Colour::Blue, Weight::Medium),
    });
    insta::assert_snapshot!("arrow_curved_blue", art(&image));
}

#[test]
fn pen_orange() {
    let image = render(&Shape::pen(
        vec![
            point(8, 54),
            point(18, 36),
            point(30, 48),
            point(42, 24),
            point(56, 34),
        ],
        Style::new(Colour::Orange, Weight::Medium),
    ));
    insta::assert_snapshot!("pen_orange", art(&image));
}

#[test]
fn pen_thick_green() {
    let image = render(&Shape::pen(
        vec![
            point(8, 16),
            point(20, 48),
            point(34, 20),
            point(48, 52),
            point(56, 30),
        ],
        Style::new(Colour::Green, Weight::Thick),
    ));
    insta::assert_snapshot!("pen_thick_green", art(&image));
}

#[test]
fn highlight_yellow() {
    let image = render(&Shape::highlight(
        vec![point(8, 28), point(56, 28)],
        Style::new(Colour::Yellow, Weight::Medium),
    ));
    insta::assert_snapshot!("highlight_yellow", art(&image));
}

#[test]
fn highlight_blue() {
    let image = render(&Shape::highlight(
        vec![point(8, 40), point(32, 40), point(56, 44)],
        Style::new(Colour::Blue, Weight::Thin),
    ));
    insta::assert_snapshot!("highlight_blue", art(&image));
}

#[test]
fn text_red() {
    let image = render(&Shape::text(point(2, 10), "Hi 1", Colour::Red));
    insta::assert_snapshot!("text_red", art(&image));
}

#[test]
fn text_white() {
    let image = render(&Shape::text(point(2, 10), "Dark", Colour::White));
    insta::assert_snapshot!("text_white", art(&image));
}

#[test]
fn text_blue() {
    let image = render(&Shape::text(point(2, 36), "Save", Colour::Blue));
    insta::assert_snapshot!("text_blue", art(&image));
}

#[test]
fn step_one_red() {
    let image = render(&Shape::step(point(32, 32), 1, Colour::Red));
    insta::assert_snapshot!("step_one_red", art(&image));
}

#[test]
fn step_two_blue() {
    let image = render(&Shape::step(point(32, 32), 2, Colour::Blue));
    insta::assert_snapshot!("step_two_blue", art(&image));
}

#[test]
fn blur_pattern() {
    let frozen = pattern(64, 64);
    let image = render_over(&Shape::blur(rect(10, 10, 44, 44)), &frozen);
    insta::assert_snapshot!("blur_checker", art(&image));
}

#[test]
fn crop_document() {
    let mut document = Document::new(pattern(64, 64));
    document
        .add(Shape::rect(
            rect(20, 20, 30, 20),
            Style::new(Colour::Blue, Weight::Medium),
        ))
        .expect("add");
    document.crop_to(rect(12, 12, 40, 32)).expect("crop");
    let image = document.render().expect("render");
    insta::assert_snapshot!("crop_document", art(&image));
}

#[test]
fn layered_document() {
    let mut document = Document::new(pattern(64, 64));
    document
        .add(Shape::rect(
            rect(8, 8, 48, 40),
            Style::new(Colour::Blue, Weight::Medium),
        ))
        .expect("add");
    document
        .add(Shape::arrow(
            point(14, 54),
            point(50, 16),
            Style::new(Colour::Red, Weight::Medium),
        ))
        .expect("add");
    document
        .add(Shape::text(point(12, 12), "Hi", Colour::Red))
        .expect("add");
    document
        .add(Shape::step(point(50, 18), 1, Colour::Green))
        .expect("add");
    document
        .add(Shape::blur(rect(30, 30, 24, 24)))
        .expect("add");
    let image = document.render().expect("render");
    insta::assert_snapshot!("layered_document", art(&image));
}

#[test]
fn tolerance_helper_is_used_by_pixels() {
    let left = Rgba::from_hex("#e5484d").expect("hex");
    assert_eq!(left, Rgba::new(229, 72, 77, 255));
    assert_eq!(left.to_hex(), "#e5484d");
}
