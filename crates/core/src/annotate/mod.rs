pub mod canvas;
pub mod command;
pub mod document;
pub mod raster;
pub mod shape;

pub use canvas::{fingerprint, Canvas, Rgba, Tolerance};
pub use command::{Command, CommandError, History};
pub use document::Document;
pub use raster::{crop_image, encode_png, Raster};
pub use shape::{
    half, inflate, label_rect, label_text_origin, points_bounds, smooth, step_rect, text_advance,
    text_scale, text_size, Annotation, Colour, Shape, ShapeId, Style, Weight, BLUR_BLOCK,
    CORNER_RADIUS, DOT_LARGE, DOT_MEDIUM, DOT_SMALL, LABEL_PAD_X, LABEL_PAD_Y, LABEL_RADIUS,
    STEP_DIAMETER, STEP_RADIUS, STEP_SIZE, STROKE_MEDIUM, STROKE_THICK, STROKE_THIN, TEXT_SIZE,
};
