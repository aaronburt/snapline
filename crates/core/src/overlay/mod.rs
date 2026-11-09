use serde::{Deserialize, Serialize};

pub mod snap;
pub mod state;

pub use snap::{SnapCandidate, SnapMode, SnapSet, SnapState};
pub use state::{Effect, Input, Overlay, Selection, State, Tool};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}
