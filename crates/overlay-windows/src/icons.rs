use std::collections::HashMap;

use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_F;
use windows::Win32::Graphics::Direct2D::ID2D1SvgDocument;
use windows::Win32::System::Com::IStream;
use windows::Win32::UI::Shell::SHCreateMemStream;

use crate::d2d::{palette_hex, Gpu};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Icon {
    Move,
    Rect,
    Ellipse,
    Arrow,
    Pen,
    Highlighter,
    Text,
    Step,
    Blur,
    Crop,
    Colour,
    Undo,
    Redo,
    Copy,
    Save,
    Pin,
    Upload,
    Record,
    Gif,
    Cancel,
    PanelRegion,
    PanelWindow,
    PanelScreen,
    PanelOpen,
    PanelGear,
    PanelLink,
    PanelMarks,
}

impl Icon {
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::Move => "M5 3l14 7-6 2-2 6z",
            Self::Rect => "M4 6h16v12H4z",
            Self::Ellipse => "M12 19c4.4 0 8-3.1 8-7s-3.6-7-8-7-8 3.1-8 7 3.6 7 8 7z",
            Self::Arrow => "M5 19L19 5M10 5h9v9",
            Self::Pen => "M3 17c3-4 5 2 8-2s4-9 7-6 1 6 3 6",
            Self::Highlighter => "M9 14l-4 4v2h6l2-2M9 14l8-8 4 4-8 8zM9 14l4 4",
            Self::Text => "M5 6V4h14v2M12 4v16M9 20h6",
            Self::Step => "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM11 9l2-1v8",
            Self::Blur => "M4 4h4v4H4zM12 4h4v4h-4zM8 8h4v4H8zM16 8h4v4h-4zM4 12h4v4H4zM12 12h4v4h-4zM8 16h4v4H8zM16 16h4v4h-4z",
            Self::Crop => "M6 2v16h16M2 6h16v16",
            Self::Colour => "M16 3l5 5-2.5 2.5-5-5zM13.5 5.5L5 14v5h5l8.5-8.5",
            Self::Undo => "M9 14L4 9l5-5M4 9h11a5 5 0 0 1 0 10h-3",
            Self::Redo => "M15 14l5-5-5-5M20 9H9a5 5 0 0 0 0 10h3",
            Self::Copy => "M9 9h11v11H9zM5 15H4V4h11v1",
            Self::Save => "M12 4v11M7 10l5 5 5-5M4 20h16",
            Self::Pin => "M9 4h6l-1 6 3 3H7l3-3zM12 13v8",
            Self::Upload => "M12 15V4M7 9l5-5 5 5M4 15v4a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-4",
            Self::Record => "M12 20a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM12 12h.01",
            Self::Gif => "M4 5h16v14H4zM10 9l5 3-5 3z",
            Self::Cancel => "M6 6l12 12M18 6L6 18",
            Self::PanelRegion => "M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M9 9h6v6H9z",
            Self::PanelWindow => "M3 6a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM3 9h18",
            Self::PanelScreen => "M3 5h18v11H3zM8 20h8M12 16v4",
            Self::PanelOpen => "M14 4h6v6M20 4l-8 8M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
            Self::PanelGear => "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
            Self::PanelLink => "M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1",
            Self::PanelMarks => "M6.75 9.75v-3h3M14.25 6.75h3v3M17.25 14.25v3h-3M9.75 17.25h-3v-3",
        }
    }

    pub(crate) const fn stroke_width(self) -> f32 {
        match self {
            Self::Upload => 1.9,
            Self::Cancel => 1.8,
            Self::PanelMarks => 1.65,
            Self::PanelRegion
            | Self::PanelWindow
            | Self::PanelScreen
            | Self::PanelOpen
            | Self::PanelGear
            | Self::PanelLink
            | Self::Copy => 1.6,
            _ => 1.7,
        }
    }
}

pub(crate) struct IconCache {
    entries: HashMap<(Icon, u32), (IStream, ID2D1SvgDocument)>,
}

impl IconCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn get(&mut self, gpu: &Gpu, icon: Icon, rgba: [f32; 4]) -> Option<ID2D1SvgDocument> {
        let packed = pack(rgba);
        if let Some((_, document)) = self.entries.get(&(icon, packed)) {
            return Some(document.clone());
        }
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\"><path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg>",
            icon.path(),
            palette_hex(rgba),
            icon.stroke_width()
        );
        // SAFETY: the byte slice outlives the call and the stream owns a copy.
        let stream = unsafe { SHCreateMemStream(Some(svg.as_bytes())) }?;
        let viewport = D2D_SIZE_F {
            width: 24.0,
            height: 24.0,
        };
        // SAFETY: the stream is a live seekable stream of valid SVG bytes.
        let document = match unsafe { gpu.context.CreateSvgDocument(&stream, viewport) } {
            Ok(document) => document,
            Err(_) => return None,
        };
        self.entries
            .insert((icon, packed), (stream, document.clone()));
        Some(document)
    }
}

fn pack(rgba: [f32; 4]) -> u32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(rgba[0]) << 16) | (channel(rgba[1]) << 8) | channel(rgba[2])
}

pub(crate) fn draw(
    cache: &mut IconCache,
    gpu: &Gpu,
    icon: Icon,
    rgba: [f32; 4],
    centre_x: f32,
    centre_y: f32,
    size: f32,
) {
    let Some(document) = cache.get(gpu, icon, rgba) else {
        return;
    };
    let scale = size / 24.0;
    let mut previous = Default::default();
    // SAFETY: the device context is live and the transform out pointer is valid.
    unsafe { gpu.context.GetTransform(&mut previous) };
    let mut transform = previous;
    transform.M11 = scale;
    transform.M12 = 0.0;
    transform.M21 = 0.0;
    transform.M22 = scale;
    transform.M31 = centre_x - size / 2.0;
    transform.M32 = centre_y - size / 2.0;
    // SAFETY: the transform is a plain matrix value and the document is live.
    unsafe {
        gpu.context.SetTransform(&transform);
        gpu.context.DrawSvgDocument(&document);
        gpu.context.SetTransform(&previous);
    }
}
