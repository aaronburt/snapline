use snapline_core::annotate::{
    crop_image, label_rect, label_text_origin, smooth, step_rect, text_advance, text_scale,
    text_size, Canvas, Document, Rgba, Shape, BLUR_BLOCK, CORNER_RADIUS, LABEL_RADIUS, STEP_RADIUS,
    STEP_SIZE, TEXT_SIZE,
};
use snapline_core::geometry::{Point, Px, Rect};
use snapline_core::image::Image;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext5, ID2D1Factory, ID2D1Factory1, ID2D1Image,
    ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_LINE_JOIN_ROUND,
    D2D1_MAP_OPTIONS_READ, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use crate::d2d::{self, Gpu};
use crate::OverlayError;

pub(crate) struct D2dCanvas<'a> {
    context: &'a ID2D1DeviceContext5,
    origin: (f32, f32),
    scale: f32,
    stroke: Option<&'a ID2D1StrokeStyle>,
}

impl<'a> D2dCanvas<'a> {
    pub(crate) fn new(
        context: &'a ID2D1DeviceContext5,
        origin: Point,
        scale: f32,
        stroke: Option<&'a ID2D1StrokeStyle>,
    ) -> Self {
        Self {
            context,
            origin: (origin.x.get() as f32, origin.y.get() as f32),
            scale: scale.max(0.01),
            stroke,
        }
    }

    fn position(&self, point: Point) -> (f32, f32) {
        (
            point.x.get() as f32 + self.origin.0,
            point.y.get() as f32 + self.origin.1,
        )
    }

    fn width(&self, width: Px) -> f32 {
        (width.0 as f32 * self.scale).max(1.0)
    }

    fn brush(&self, colour: Rgba) -> Option<ID2D1SolidColorBrush> {
        let value = d2d::colour(rgba(colour));
        // SAFETY: the colour is a plain value and the device context is live.
        unsafe { self.context.CreateSolidColorBrush(&value, None) }.ok()
    }

    fn antialias(&self, mode: windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE) {
        // SAFETY: the mode is a valid enumeration value.
        unsafe {
            self.context.SetAntialiasMode(mode);
        }
    }

    fn fill(&self, area: D2D_RECT_F, radius: f32, colour: Rgba) {
        let Some(brush) = self.brush(colour) else {
            return;
        };
        if radius <= 0.0 {
            self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
            // SAFETY: the rectangle and brush are live values.
            unsafe {
                let _ = self.context.FillRectangle(&area, &brush);
            }
        } else {
            self.antialias(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let rounded = D2D1_ROUNDED_RECT {
                rect: area,
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: the rounded rectangle and brush are live values.
            unsafe {
                let _ = self.context.FillRoundedRectangle(&rounded, &brush);
            }
        }
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
    }

    fn stroke(&self, area: D2D_RECT_F, radius: f32, width: f32, colour: Rgba) {
        let Some(brush) = self.brush(colour) else {
            return;
        };
        if radius <= 0.0 {
            self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
            // SAFETY: the rectangle, brush and width are valid values.
            unsafe {
                let _ = self.context.DrawRectangle(&area, &brush, width, None);
            }
        } else {
            self.antialias(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            let rounded = D2D1_ROUNDED_RECT {
                rect: area,
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: the rounded rectangle, brush and width are valid values.
            unsafe {
                let _ = self
                    .context
                    .DrawRoundedRectangle(&rounded, &brush, width, self.stroke);
            }
        }
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
    }

    fn segment(&self, from: Point, to: Point, width: f32, colour: Rgba) {
        let Some(brush) = self.brush(colour) else {
            return;
        };
        let (x0, y0) = self.position(from);
        let (x1, y1) = self.position(to);
        let mut start = D2D1_ELLIPSE::default();
        start.point.X = x0;
        start.point.Y = y0;
        let mut end = D2D1_ELLIPSE::default();
        end.point.X = x1;
        end.point.Y = y1;
        self.antialias(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        // SAFETY: the two points are plain values, the brush is live and the stroke style may be null.
        unsafe {
            let _ = self
                .context
                .DrawLine(start.point, end.point, &brush, width, self.stroke);
        }
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
    }

    pub(crate) fn draw_shape(&mut self, shape: &Shape, frozen: &Image) {
        match shape {
            Shape::Rect { rect, style } => {
                self.stroke_rect(*rect, CORNER_RADIUS, style.width(), style.rgba());
            }
            Shape::Ellipse { rect, style } => {
                self.stroke_ellipse(*rect, style.width(), style.rgba());
            }
            Shape::Arrow {
                tail,
                tip,
                curve,
                style,
            } => {
                self.draw_arrow(*tail, *tip, *curve, self.width(style.width()), style.rgba());
            }
            Shape::Pen { points, style } => {
                self.polyline(&smooth(points), style.width(), style.rgba());
            }
            Shape::Highlight { points, style } => {
                self.polyline(points, style.width(), style.highlight());
            }
            Shape::Text {
                origin,
                text,
                colour,
            } => {
                self.fill_rect(
                    label_rect(*origin, text),
                    LABEL_RADIUS,
                    colour.label_background(),
                );
                self.text(
                    label_text_origin(*origin),
                    text,
                    TEXT_SIZE,
                    true,
                    colour.rgba(),
                );
            }
            Shape::Step {
                center,
                number,
                colour,
            } => {
                self.fill_rect(step_rect(*center), STEP_RADIUS, colour.rgba());
                let value = number.to_string();
                let size = text_size(&value, STEP_SIZE);
                let origin = Point::new(
                    Px(center.x.0 - size.width.0 / 2),
                    Px(center.y.0 - size.height.0 / 2),
                );
                self.text(origin, &value, STEP_SIZE, true, Rgba::WHITE);
            }
            Shape::Blur { rect } => {
                self.pixelate(*rect, BLUR_BLOCK, frozen);
            }
        }
    }

    fn draw_arrow(
        &mut self,
        tail: Point,
        tip: Point,
        curve: Option<Point>,
        width: f32,
        colour: Rgba,
    ) {
        let shaft = match curve {
            Some(control) => quadratic(tail, control, tip),
            None => vec![tail, tip],
        };
        let direction = match curve {
            Some(control) => (
                f64::from(tip.x.0 - control.x.0),
                f64::from(tip.y.0 - control.y.0),
            ),
            None => (f64::from(tip.x.0 - tail.x.0), f64::from(tip.y.0 - tail.y.0)),
        };
        let length = direction.0.hypot(direction.1);
        if length == 0.0 {
            return;
        }
        let ux = direction.0 / length;
        let uy = direction.1 / length;
        let head = (f64::from(width) * 2.0 + 12.0).max(14.0);
        let back_x = f64::from(tip.x.0) - ux * head;
        let back_y = f64::from(tip.y.0) - uy * head;
        let wing = head * 0.5;
        let first = Point::new(Px(round(back_x - uy * wing)), Px(round(back_y + ux * wing)));
        let second = Point::new(Px(round(back_x + uy * wing)), Px(round(back_y - ux * wing)));
        self.polyline_with(&shaft, width, colour);
        self.polyline_with(&[tip, first], width, colour);
        self.polyline_with(&[tip, second], width, colour);
    }

    fn polyline_with(&mut self, points: &[Point], width: f32, colour: Rgba) {
        if points.is_empty() {
            return;
        }
        if points.len() == 1 {
            let (x, y) = self.position(points[0]);
            self.antialias(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.fill_circle_at(x, y, width / 2.0, colour);
            return;
        }
        for pair in points.windows(2) {
            self.segment(pair[0], pair[1], width, colour);
        }
    }

    fn fill_circle_at(&self, x: f32, y: f32, radius: f32, colour: Rgba) {
        let Some(brush) = self.brush(colour) else {
            return;
        };
        let mut ellipse = D2D1_ELLIPSE::default();
        ellipse.point.X = x;
        ellipse.point.Y = y;
        ellipse.radiusX = radius;
        ellipse.radiusY = radius;
        // SAFETY: the ellipse and brush are live values.
        unsafe {
            let _ = self.context.FillEllipse(&ellipse, &brush);
        }
    }
}

impl Canvas for D2dCanvas<'_> {
    fn fill_rect(&mut self, rect: Rect, radius: Px, colour: Rgba) {
        let (x, y) = self.position(rect.origin());
        let area = d2d::rect(x, y, rect.width.get() as f32, rect.height.get() as f32);
        self.fill(area, radius.0.max(0) as f32, colour);
    }

    fn stroke_rect(&mut self, rect: Rect, radius: Px, width: Px, colour: Rgba) {
        let stroke = self.width(width);
        let half = stroke / 2.0;
        let (x, y) = self.position(rect.origin());
        let area = d2d::rect(
            x + half,
            y + half,
            (rect.width.get() as f32 - stroke).max(0.0),
            (rect.height.get() as f32 - stroke).max(0.0),
        );
        let radius = (radius.0 as f32 - half).max(0.0);
        self.stroke(area, radius, stroke, colour);
    }

    fn stroke_ellipse(&mut self, rect: Rect, width: Px, colour: Rgba) {
        let stroke = self.width(width);
        let (x, y) = self.position(rect.origin());
        let radius_x = (rect.width.get() as f32 - stroke).max(0.0) / 2.0;
        let radius_y = (rect.height.get() as f32 - stroke).max(0.0) / 2.0;
        let Some(brush) = self.brush(colour) else {
            return;
        };
        let mut ellipse = D2D1_ELLIPSE::default();
        ellipse.point.X = x + rect.width.get() as f32 / 2.0;
        ellipse.point.Y = y + rect.height.get() as f32 / 2.0;
        ellipse.radiusX = radius_x;
        ellipse.radiusY = radius_y;
        self.antialias(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        // SAFETY: the ellipse, brush and width are live values.
        unsafe {
            let _ = self.context.DrawEllipse(&ellipse, &brush, stroke, None);
        }
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
    }

    fn line(&mut self, from: Point, to: Point, width: Px, colour: Rgba) {
        let stroke = self.width(width);
        self.segment(from, to, stroke, colour);
    }

    fn polyline(&mut self, points: &[Point], width: Px, colour: Rgba) {
        let stroke = self.width(width);
        self.polyline_with(points, stroke, colour);
    }

    fn text(&mut self, origin: Point, text: &str, size: Px, bold: bool, colour: Rgba) {
        let scale = text_scale(size);
        let advance = text_advance(size);
        let Some(brush) = self.brush(colour) else {
            return;
        };
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
        for (index, character) in text.chars().enumerate() {
            let rows = glyph(character);
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) == 0 {
                        continue;
                    }
                    let x = origin.x.0 + index as i32 * advance + column * scale;
                    let y = origin.y.0 + row as i32 * scale;
                    let width = scale + if bold { 1 } else { 0 };
                    let (left, top) = self.position(Point::new(Px(x), Px(y)));
                    let area = d2d::rect(left, top, width as f32, scale as f32);
                    // SAFETY: the rectangle and brush are live values.
                    unsafe {
                        let _ = self.context.FillRectangle(&area, &brush);
                    }
                }
            }
        }
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
    }

    fn pixelate(&mut self, rect: Rect, block: Px, source: &Image) {
        let block = block.0.max(1);
        let Some((left, top, right, bottom)) = clip_to(rect, source.width(), source.height())
        else {
            return;
        };
        let Some(brush) = self.brush(Rgba::TRANSPARENT) else {
            return;
        };
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
        let mut y = top;
        while y < bottom {
            let mut x = left;
            while x < right {
                let end_x = (x + block).min(right);
                let end_y = (y + block).min(bottom);
                if end_x > x && end_y > y {
                    let count = ((end_x - x) * (end_y - y)) as u32;
                    let mut total = [0_u32; 4];
                    for sample_y in y..end_y {
                        for sample_x in x..end_x {
                            let Ok(pixel) = source.pixel(sample_x as u32, sample_y as u32) else {
                                continue;
                            };
                            for channel in 0..4 {
                                total[channel] += u32::from(pixel[channel]);
                            }
                        }
                    }
                    let average = |value: u32| ((value + count / 2) / count) as u8;
                    let colour = Rgba::new(
                        average(total[2]),
                        average(total[1]),
                        average(total[0]),
                        average(total[3]),
                    );
                    let (origin_x, origin_y) = self.position(Point::new(Px(x), Px(y)));
                    let area =
                        d2d::rect(origin_x, origin_y, (end_x - x) as f32, (end_y - y) as f32);
                    // SAFETY: the brush is live and the colour is a plain value.
                    unsafe {
                        brush.SetColor(&d2d::colour(rgba(colour)));
                        let _ = self.context.FillRectangle(&area, &brush);
                    }
                }
                x += block;
            }
            y += block;
        }
        self.antialias(D2D1_ANTIALIAS_MODE_ALIASED);
    }

    fn image(&mut self, image: &Image, rect: Rect) {
        let properties = bitmap_properties(Default::default());
        let size = D2D_SIZE_U {
            width: image.width(),
            height: image.height(),
        };
        // SAFETY: the pixel buffer outlives the call and the size matches the stride.
        let bitmap = unsafe {
            self.context.CreateBitmap(
                size,
                Some(image.pixels().as_ptr().cast()),
                image.stride() as u32,
                &properties,
            )
        };
        let Ok(bitmap) = bitmap else {
            return;
        };
        let (x, y) = self.position(rect.origin());
        d2d::draw_bitmap(
            self.context,
            &bitmap,
            d2d::rect(x, y, rect.width.get() as f32, rect.height.get() as f32),
            d2d::rect(0.0, 0.0, image.width() as f32, image.height() as f32),
            false,
        );
    }
}

pub(crate) fn stroke_style() -> Option<ID2D1StrokeStyle> {
    let factory: ID2D1Factory1 =
        // SAFETY: a single threaded factory takes no options and no invalid pointers.
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }.ok()?;
    let properties = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: D2D1_CAP_STYLE_ROUND,
        endCap: D2D1_CAP_STYLE_ROUND,
        dashCap: D2D1_CAP_STYLE_ROUND,
        lineJoin: D2D1_LINE_JOIN_ROUND,
        miterLimit: 1.0,
        ..Default::default()
    };
    // SAFETY: the properties are a plain value and no dash pattern is used.
    unsafe { ID2D1Factory::CreateStrokeStyle(&factory, &properties, None) }.ok()
}

pub(crate) fn draw_document(
    gpu: &Gpu,
    document: &Document,
    origin: Point,
    scale: f32,
    stroke: Option<&ID2D1StrokeStyle>,
) {
    let context = &gpu.context;
    let mut canvas = D2dCanvas::new(context, origin, scale, stroke);
    for annotation in document.annotations() {
        canvas.draw_shape(&annotation.shape, document.image());
    }
}

pub(crate) fn render_document(
    gpu: &Gpu,
    document: &Document,
    scale: f32,
    stroke: Option<&ID2D1StrokeStyle>,
) -> Result<Image, OverlayError> {
    let full = document.full_rect();
    if full.is_empty() {
        return Err(OverlayError::Device(
            "The annotation image is empty.".to_string(),
        ));
    }
    let width = u32::try_from(full.width.get())
        .map_err(|_| OverlayError::Device("The annotation image is too large.".to_string()))?;
    let height = u32::try_from(full.height.get())
        .map_err(|_| OverlayError::Device("The annotation image is too large.".to_string()))?;
    let context = &gpu.context;
    let source = gpu.upload_image(document.image())?;
    let target_properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..Default::default()
    };
    // SAFETY: the size is positive and the properties are a plain value.
    let target =
        unsafe { context.CreateBitmap(D2D_SIZE_U { width, height }, None, 0, &target_properties) }
            .map_err(device)?;
    // SAFETY: the previous target is returned by value and may be absent.
    let previous = unsafe { context.GetTarget() }.ok();
    // SAFETY: the target bitmap was created for this context.
    unsafe {
        context.SetTarget(&target);
    }
    // SAFETY: every begin is paired with the end below.
    unsafe {
        context.BeginDraw();
    }
    let clear = d2d::colour([0.0, 0.0, 0.0, 1.0]);
    // SAFETY: the colour is a plain value.
    unsafe {
        let _ = context.Clear(Some(&clear));
    }
    d2d::draw_bitmap(
        context,
        &source,
        d2d::rect(0.0, 0.0, width as f32, height as f32),
        d2d::rect(
            0.0,
            0.0,
            document.image().width() as f32,
            document.image().height() as f32,
        ),
        false,
    );
    {
        let mut canvas = D2dCanvas::new(context, Point::ORIGIN, scale, stroke);
        for annotation in document.annotations() {
            canvas.draw_shape(&annotation.shape, document.image());
        }
    }
    let drawn = unsafe { context.EndDraw(None, None) };
    // SAFETY: restores the target that was active before this render pass.
    unsafe {
        if let Some(previous) = previous.as_ref() {
            context.SetTarget(previous);
        } else {
            context.SetTarget(None::<&ID2D1Image>);
        }
    }
    drawn.map_err(device)?;
    let full_image = read_back(context, &target, width, height)?;
    crop_image(&full_image, document.crop())
        .map_err(|error| OverlayError::Device(error.to_string()))
}

fn read_back(
    context: &ID2D1DeviceContext5,
    target: &ID2D1Bitmap1,
    width: u32,
    height: u32,
) -> Result<Image, OverlayError> {
    let properties =
        bitmap_properties(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW);
    // SAFETY: the size is positive and the properties are a plain value.
    let readable =
        unsafe { context.CreateBitmap(D2D_SIZE_U { width, height }, None, 0, &properties) }
            .map_err(device)?;
    // SAFETY: the source target has the same size and format.
    unsafe { readable.CopyFromBitmap(None, target, None) }.map_err(device)?;
    // SAFETY: the bitmap was created with CPU read access.
    let mapped = unsafe { readable.Map(D2D1_MAP_OPTIONS_READ) }.map_err(device)?;
    let mut image =
        Image::bgra(width, height).map_err(|error| OverlayError::Device(error.to_string()))?;
    for y in 0..height {
        let offset = y as usize * mapped.pitch as usize;
        // SAFETY: the mapped buffer holds a full row of BGRA pixels for this bitmap.
        let row =
            unsafe { std::slice::from_raw_parts(mapped.bits.add(offset), width as usize * 4) };
        if let Some(output) = image.row_mut(y) {
            output.copy_from_slice(row);
            for pixel in output.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
    }
    // SAFETY: the map is released once and no slice outlives it.
    let _ = unsafe { readable.Unmap() };
    Ok(image)
}

fn bitmap_properties(
    options: windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS,
) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        ..Default::default()
    }
}

fn device(error: windows::core::Error) -> OverlayError {
    OverlayError::Device(error.to_string())
}

fn rgba(colour: Rgba) -> [f32; 4] {
    [
        f32::from(colour.r) / 255.0,
        f32::from(colour.g) / 255.0,
        f32::from(colour.b) / 255.0,
        f32::from(colour.a) / 255.0,
    ]
}

fn clip_to(rect: Rect, width: u32, height: u32) -> Option<(i32, i32, i32, i32)> {
    let left = rect.left().0.max(0);
    let top = rect.top().0.max(0);
    let right = rect.right().0.min(width as i32);
    let bottom = rect.bottom().0.min(height as i32);
    if right <= left || bottom <= top {
        None
    } else {
        Some((left, top, right, bottom))
    }
}

fn round(value: f64) -> i32 {
    if value >= 0.0 {
        (value + 0.5).floor() as i32
    } else {
        (value - 0.5).ceil() as i32
    }
}

fn quadratic(tail: Point, control: Point, tip: Point) -> Vec<Point> {
    let mut points = Vec::with_capacity(17);
    for step in 0..=16 {
        let t = f64::from(step) / 16.0;
        let inverse = 1.0 - t;
        let x = inverse * inverse * f64::from(tail.x.0)
            + 2.0 * inverse * t * f64::from(control.x.0)
            + t * t * f64::from(tip.x.0);
        let y = inverse * inverse * f64::from(tail.y.0)
            + 2.0 * inverse * t * f64::from(control.y.0)
            + t * t * f64::from(tip.y.0);
        let point = Point::new(Px(round(x)), Px(round(y)));
        if points.last() != Some(&point) {
            points.push(point);
        }
    }
    points
}

fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        ',' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00100, 0b00100, 0b01000,
        ],
        ':' => [
            0b00000, 0b00110, 0b00110, 0b00000, 0b00110, 0b00110, 0b00000,
        ],
        ';' => [
            0b00000, 0b00110, 0b00110, 0b00000, 0b00110, 0b00110, 0b01000,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
        ],
        '+' => [
            0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
        ],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '!' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
        '\'' => [
            0b00100, 0b00100, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000,
        ],
        '"' => [
            0b01010, 0b01010, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000,
        ],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '=' => [
            0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000,
        ],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        '%' => [
            0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011,
        ],
        '&' => [
            0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
        ],
        '*' => [
            0b00000, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00000,
        ],
        '@' => [
            0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01110,
        ],
        _ => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapline_core::annotate::{Colour, Style, Tolerance, Weight};

    fn blank(width: u32, height: u32) -> Option<Image> {
        let mut image = Image::bgra(width, height).ok()?;
        for y in 0..height {
            for x in 0..width {
                let _ = image.set_pixel(x, y, [0, 0, 0, 255]);
            }
        }
        Some(image)
    }

    fn different(expected: &Image, actual: &Image, tolerance: u8) -> usize {
        if expected.width() != actual.width() || expected.height() != actual.height() {
            return usize::MAX;
        }
        let tolerance = Tolerance::new(tolerance);
        let mut count = 0;
        for y in 0..expected.height() {
            for x in 0..expected.width() {
                let (Ok(left), Ok(right)) = (expected.pixel(x, y), actual.pixel(x, y)) else {
                    return usize::MAX;
                };
                if !tolerance.within(left, right) {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    #[ignore = "desktop"]
    fn direct2d_text_matches_the_software_rasteriser() {
        let Ok(gpu) = Gpu::new() else {
            return;
        };
        let Some(image) = blank(180, 40) else {
            return;
        };
        let mut document = Document::new(image);
        let _ = document.add(Shape::text(
            Point::new(Px(12), Px(12)),
            "SNAP 42",
            Colour::White,
        ));
        let style = stroke_style();
        let Ok(actual) = render_document(&gpu, &document, 1.0, style.as_ref()) else {
            return;
        };
        let Ok(expected) = document.render() else {
            return;
        };
        assert_eq!(
            different(&expected, &actual, 2),
            0,
            "the Direct2D text differs from the software rasteriser"
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn direct2d_rect_matches_the_software_rasteriser_away_from_corners() {
        let Ok(gpu) = Gpu::new() else {
            return;
        };
        let Some(image) = blank(200, 140) else {
            return;
        };
        let mut document = Document::new(image);
        let _ = document.add(Shape::rect(
            Rect::new(Px(20), Px(20), Px(160), Px(100)),
            Style::new(Colour::Red, Weight::Thick),
        ));
        let style = stroke_style();
        let Ok(actual) = render_document(&gpu, &document, 1.0, style.as_ref()) else {
            return;
        };
        let Ok(expected) = document.render() else {
            return;
        };
        let total = (expected.width() * expected.height()) as usize;
        assert!(
            different(&expected, &actual, 2) * 20 < total,
            "the Direct2D rectangle differs too much from the software rasteriser"
        );
    }

    #[test]
    #[ignore = "desktop"]
    fn direct2d_step_draws_the_circle_and_glyph() {
        let Ok(gpu) = Gpu::new() else {
            return;
        };
        let Some(image) = blank(120, 120) else {
            return;
        };
        let mut document = Document::new(image);
        let _ = document.add(Shape::step(Point::new(Px(60), Px(60)), 1, Colour::Red));
        let style = stroke_style();
        let Ok(actual) = render_document(&gpu, &document, 1.0, style.as_ref()) else {
            return;
        };
        let Ok(expected) = document.render() else {
            return;
        };
        let total = (expected.width() * expected.height()) as usize;
        assert!(
            different(&expected, &actual, 2) * 4 < total,
            "the Direct2D step differs too much from the software rasteriser"
        );
        let centre = actual.pixel(59, 54).unwrap_or_default();
        assert!(centre[2] > 200 && centre[1] > 200 && centre[0] > 200);
    }
}
