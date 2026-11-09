use serde::{Deserialize, Serialize};

use crate::geometry::{Point, Rect};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SnapMode {
    #[default]
    Window,
    Control,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapCandidate {
    pub rect: Rect,
}

impl SnapCandidate {
    pub const fn new(rect: Rect) -> Self {
        Self { rect }
    }
}

impl From<Rect> for SnapCandidate {
    fn from(rect: Rect) -> Self {
        Self::new(rect)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapSet {
    pub windows: Vec<SnapCandidate>,
    pub controls: Vec<SnapCandidate>,
}

impl SnapSet {
    pub fn new(windows: Vec<Rect>, controls: Vec<Rect>) -> Self {
        Self {
            windows: windows.into_iter().map(SnapCandidate::from).collect(),
            controls: controls.into_iter().map(SnapCandidate::from).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty() && self.controls.is_empty()
    }

    pub fn candidates(&self, mode: SnapMode) -> &[SnapCandidate] {
        match mode {
            SnapMode::Window => &self.windows,
            SnapMode::Control => &self.controls,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapState {
    pub set: SnapSet,
    pub mode: SnapMode,
    pub hovered: Option<Rect>,
}

impl SnapState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_set(set: SnapSet) -> Self {
        Self {
            set,
            mode: SnapMode::Window,
            hovered: None,
        }
    }

    pub fn set_candidates(&mut self, set: SnapSet) {
        self.set = set;
        self.hovered = None;
    }

    pub fn candidates(&self) -> &[SnapCandidate] {
        self.set.candidates(self.mode)
    }

    pub fn hover_at(&self, point: Point) -> Option<Rect> {
        match self.mode {
            SnapMode::Window => self
                .set
                .windows
                .iter()
                .map(|candidate| candidate.rect)
                .find(|rect| rect.contains(point)),
            SnapMode::Control => self
                .set
                .controls
                .iter()
                .map(|candidate| candidate.rect)
                .filter(|rect| rect.contains(point))
                .min_by_key(|rect| rect.area()),
        }
    }

    pub fn update_hover(&mut self, point: Point) -> Option<Rect> {
        let hovered = self.hover_at(point);
        self.hovered = hovered;
        hovered
    }

    pub fn clear_hover(&mut self) -> Option<Rect> {
        self.hovered.take()
    }

    pub fn cycle_mode(&mut self) -> SnapMode {
        self.mode = match self.mode {
            SnapMode::Window => SnapMode::Control,
            SnapMode::Control => SnapMode::Window,
        };
        self.hovered = None;
        self.mode
    }
}

#[cfg(test)]
mod tests {
    use crate::geometry::{Px, Rect};

    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    fn set() -> SnapSet {
        SnapSet::new(
            vec![rect(0, 0, 500, 500), rect(100, 100, 200, 200)],
            vec![
                rect(0, 0, 500, 500),
                rect(100, 100, 200, 200),
                rect(120, 120, 50, 50),
            ],
        )
    }

    #[test]
    fn window_mode_uses_first_containing_window() {
        let state = SnapState::with_set(set());
        assert_eq!(state.hover_at(point(150, 150)), Some(rect(0, 0, 500, 500)));
        assert_eq!(state.hover_at(point(20, 20)), Some(rect(0, 0, 500, 500)));
        assert_eq!(state.hover_at(point(700, 700)), None);
    }

    #[test]
    fn control_mode_uses_smallest_containing_control() {
        let mut state = SnapState::with_set(set());
        state.cycle_mode();
        assert_eq!(
            state.hover_at(point(150, 150)),
            Some(rect(120, 120, 50, 50))
        );
        assert_eq!(
            state.hover_at(point(250, 250)),
            Some(rect(100, 100, 200, 200))
        );
        assert_eq!(state.hover_at(point(20, 20)), Some(rect(0, 0, 500, 500)));
        assert_eq!(state.hover_at(point(700, 700)), None);
    }

    #[test]
    fn tab_cycles_window_and_control() {
        let mut state = SnapState::with_set(set());
        assert_eq!(state.mode, SnapMode::Window);
        assert_eq!(state.cycle_mode(), SnapMode::Control);
        assert_eq!(state.mode, SnapMode::Control);
        assert_eq!(state.cycle_mode(), SnapMode::Window);
        assert_eq!(state.mode, SnapMode::Window);
    }

    #[test]
    fn update_and_clear_hover_track_the_pointer() {
        let mut state = SnapState::with_set(set());
        assert_eq!(
            state.update_hover(point(150, 150)),
            Some(rect(0, 0, 500, 500))
        );
        assert_eq!(state.hovered, Some(rect(0, 0, 500, 500)));
        assert_eq!(state.update_hover(point(700, 700)), None);
        assert_eq!(state.hovered, None);
        state.update_hover(point(150, 150));
        assert_eq!(state.clear_hover(), Some(rect(0, 0, 500, 500)));
        assert_eq!(state.hovered, None);
    }

    #[test]
    fn setting_candidates_clears_hover() {
        let mut state = SnapState::with_set(set());
        state.update_hover(point(150, 150));
        state.set_candidates(SnapSet::new(vec![rect(900, 900, 10, 10)], vec![]));
        assert_eq!(state.hovered, None);
        assert_eq!(state.hover_at(point(150, 150)), None);
        assert_eq!(
            state.hover_at(point(905, 905)),
            Some(rect(900, 900, 10, 10))
        );
    }

    #[test]
    fn switching_mode_clears_hover_until_the_pointer_moves() {
        let mut state = SnapState::with_set(set());
        state.update_hover(point(150, 150));
        assert_eq!(state.hovered, Some(rect(0, 0, 500, 500)));
        state.cycle_mode();
        assert_eq!(state.hovered, None);
    }

    #[test]
    fn empty_set_has_no_candidates() {
        let state = SnapState::new();
        assert!(state.set.is_empty());
        assert_eq!(state.hover_at(point(0, 0)), None);
        assert_eq!(state.candidates().len(), 0);
    }
}
