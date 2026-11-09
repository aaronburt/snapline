#![doc = "Native overlay windows for Snapline, drawn with Win32, Direct2D, DirectComposition and DirectWrite. This crate owns the capture overlay, annotation toolbar, pin, ruler, colour picker and recording border. It may use unsafe code and must never host a webview. The public surface is `OverlayHostImpl`, which runs one dedicated overlay thread per request and returns an `OverlayResult`; the matching core port that a later task must add is `OverlayHost::select_region(&self, OverlayRequest) -> Result<OverlayResult, OverlayError>` plus an edit entry point."]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::default_trait_access,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_ref_mut,
    clippy::too_many_lines,
    clippy::unused_self
)]

pub mod canvas_d2d;
pub mod d2d;
pub mod hints;
pub mod icons;
pub mod magnifier;
pub mod picker;
pub mod pin;
pub mod port_impl;
pub mod readout;
#[allow(dead_code)]
pub mod render;
pub mod ruler;
pub mod text_input;
pub mod thread;
pub mod tokens;
pub mod toolbar;
pub mod tools;
pub mod window;

use snapline_core::geometry::Rect;
use snapline_core::image::Image;
use snapline_core::overlay::{Rgb, SnapSet, Tool};
use snapline_core::ports::FrozenDesktop;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn palette(self) -> &'static tokens::Palette {
        match self {
            Self::Dark => &tokens::DARK,
            Self::Light => &tokens::LIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    Copy,
    Save,
    Pin,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagnifierZoom {
    X2,
    X4,
    X8,
}

impl MagnifierZoom {
    pub const fn factor(self) -> i32 {
        match self {
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagnifierSettings {
    pub enabled: bool,
    pub zoom: MagnifierZoom,
}

impl Default for MagnifierSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            zoom: MagnifierZoom::X4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationColour {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    White,
}

impl AnnotationColour {
    pub const ALL: [Self; 6] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::White,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::White => "White",
        }
    }

    pub fn rgba(self, palette: &tokens::Palette) -> [f32; 4] {
        match self {
            Self::Red => palette.red,
            Self::Orange => palette.orange,
            Self::Yellow => palette.yellow,
            Self::Green => palette.green,
            Self::Blue => palette.blue,
            Self::White => palette.white,
        }
    }

    pub const fn rgb(self) -> Rgb {
        match self {
            Self::Red => Rgb::new(0xe5, 0x48, 0x4d),
            Self::Orange => Rgb::new(0xf7, 0x6b, 0x15),
            Self::Yellow => Rgb::new(0xff, 0xd6, 0x0a),
            Self::Green => Rgb::new(0x30, 0xa4, 0x6c),
            Self::Blue => Rgb::new(0x00, 0x90, 0xff),
            Self::White => Rgb::new(0xff, 0xff, 0xff),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeSize {
    Thin,
    Medium,
    Thick,
}

impl StrokeSize {
    pub const ALL: [Self; 3] = [Self::Thin, Self::Medium, Self::Thick];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Thin => "Thin",
            Self::Medium => "Medium",
            Self::Thick => "Thick",
        }
    }

    pub const fn dot(self) -> i32 {
        match self {
            Self::Thin => 4,
            Self::Medium => 7,
            Self::Thick => 11,
        }
    }

    pub const fn width(self) -> i32 {
        match self {
            Self::Thin => 2,
            Self::Medium => 4,
            Self::Thick => 8,
        }
    }
}

impl Default for StrokeSize {
    fn default() -> Self {
        Self::Medium
    }
}

#[derive(Debug, Clone)]
pub struct SelectRequest {
    pub desktop: FrozenDesktop,
    pub annotate: bool,
    pub snap: SnapSet,
    pub magnifier: MagnifierSettings,
    pub full_screen_crosshair: bool,
    pub theme: Theme,
}

impl SelectRequest {
    pub fn new(desktop: FrozenDesktop) -> Self {
        Self {
            desktop,
            annotate: false,
            snap: SnapSet::default(),
            magnifier: MagnifierSettings::default(),
            full_screen_crosshair: false,
            theme: Theme::Dark,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditRequest {
    pub image: Image,
    pub magnifier: MagnifierSettings,
    pub theme: Theme,
}

impl EditRequest {
    pub fn new(image: Image) -> Self {
        Self {
            image,
            magnifier: MagnifierSettings::default(),
            theme: Theme::Dark,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayResult {
    pub action: Option<OverlayAction>,
    pub selection: Rect,
    pub monitor: Option<u32>,
    pub tool: Tool,
    pub colour: Rgb,
    pub image: Option<Image>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OverlayError {
    #[error("There is no monitor to draw the overlay on.")]
    NoMonitors,
    #[error("The overlay thread could not be started.")]
    Thread,
    #[error("The overlay was cancelled.")]
    Cancelled,
    #[error("The overlay could not use the graphics device: {0}")]
    Device(String),
}

pub struct OverlayHostImpl;

impl Default for OverlayHostImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayHostImpl {
    pub const fn new() -> Self {
        Self
    }

    pub fn select_region(&self, req: SelectRequest) -> Result<OverlayResult, OverlayError> {
        if req.desktop.monitors.is_empty() {
            return Err(OverlayError::NoMonitors);
        }
        thread::run_select(req)
    }

    pub fn edit_image(&self, req: EditRequest) -> Result<OverlayResult, OverlayError> {
        thread::run_edit(req)
    }
}
