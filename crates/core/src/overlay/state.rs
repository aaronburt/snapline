use serde::{Deserialize, Serialize};

use crate::geometry::{Direction, Grip, Point, Px, Rect, Size};
use crate::overlay::snap::{SnapSet, SnapState};
use crate::overlay::Rgb;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    Move,
    Rect,
    Ellipse,
    Arrow,
    Pen,
    #[default]
    Highlighter,
    Text,
    Step,
    Blur,
    Crop,
    Colour,
}

impl Tool {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
            Self::Arrow => "arrow",
            Self::Pen => "pen",
            Self::Highlighter => "highlight",
            Self::Text => "text",
            Self::Step => "step",
            Self::Blur => "blur",
            Self::Crop => "crop",
            Self::Colour => "color",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selection {
    pub rect: Rect,
}

impl Selection {
    pub const fn new(rect: Rect) -> Self {
        Self { rect }
    }

    pub const fn origin(self) -> Point {
        self.rect.origin()
    }

    pub const fn size(self) -> Size {
        self.rect.size()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Redraw(Rect),
    Finish(Selection),
    Cancel,
    CopyColour(Rgb),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Hovering { snap: SnapState },
    Dragging { from: Point },
    Adjusting { rect: Rect, grip: Grip },
    Annotating { rect: Rect, tool: Tool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Move(Point),
    Press(Point),
    Release(Point),
    RightClick,
    Escape,
    Tab,
    Arrow {
        direction: Direction,
        shift: bool,
        alt: bool,
    },
    SetTool(Tool),
    Copy,
    Save,
    Upload,
    Cancel,
    SnapCandidates(SnapSet),
    CopyColour(Rgb),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub state: State,
    pub pointer: Option<Point>,
    pub active_grip: Grip,
    pub tool: Tool,
    pub annotate: bool,
    pub snap: SnapState,
}

impl Overlay {
    pub fn new() -> Self {
        Self::with_annotate(false)
    }

    pub fn with_annotate(annotate: bool) -> Self {
        Self {
            state: State::Hovering {
                snap: SnapState::new(),
            },
            pointer: None,
            active_grip: Grip::SouthEast,
            tool: Tool::default(),
            annotate,
            snap: SnapState::new(),
        }
    }

    pub fn selected_rect(&self) -> Option<Rect> {
        match &self.state {
            State::Adjusting { rect, .. } | State::Annotating { rect, .. } => Some(*rect),
            State::Hovering { .. } | State::Dragging { .. } => None,
        }
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selected_rect().map(Selection::new)
    }

    pub fn on(mut self, input: Input) -> (Self, Vec<Effect>) {
        let mut effects = Vec::new();
        match input {
            Input::Move(point) => self.on_move(point, &mut effects),
            Input::Press(point) => self.on_press(point, &mut effects),
            Input::Release(point) => self.on_release(point, &mut effects),
            Input::RightClick => self.on_right_click(&mut effects),
            Input::Escape | Input::Cancel => self.cancel(&mut effects),
            Input::Tab => self.on_tab(&mut effects),
            Input::Arrow {
                direction,
                shift,
                alt,
            } => self.on_arrow(direction, shift, alt, &mut effects),
            Input::SetTool(tool) => self.on_set_tool(tool, &mut effects),
            Input::Copy | Input::Save | Input::Upload => self.on_commit(&mut effects),
            Input::SnapCandidates(set) => self.on_snap_candidates(set, &mut effects),
            Input::CopyColour(rgb) => self.on_copy_colour(rgb, &mut effects),
        }
        (self, effects)
    }

    fn resting_state(&self) -> State {
        let mut snap = self.snap.clone();
        snap.clear_hover();
        State::Hovering { snap }
    }

    fn sync_snap(&mut self) {
        if let State::Hovering { snap } = &self.state {
            self.snap = snap.clone();
        }
    }

    fn on_move(&mut self, point: Point, effects: &mut Vec<Effect>) {
        let previous = self.pointer.replace(point);
        match &mut self.state {
            State::Hovering { snap } => {
                let old = snap.hovered;
                snap.update_hover(point);
                let new = snap.hovered;
                self.snap = snap.clone();
                push_redraw(old, new, effects);
            }
            State::Dragging { from } => {
                let from = *from;
                let old = Rect::from_points(from, previous.unwrap_or(point));
                let new = Rect::from_points(from, point);
                push_redraw(Some(old), Some(new), effects);
            }
            State::Adjusting { rect, grip } => {
                let old = *rect;
                let new = match grip {
                    Grip::Body => rect.translated(point - previous.unwrap_or(point)),
                    Grip::NorthWest
                    | Grip::North
                    | Grip::NorthEast
                    | Grip::East
                    | Grip::SouthEast
                    | Grip::South
                    | Grip::SouthWest
                    | Grip::West => grip.resize_to(*rect, point),
                };
                *rect = new;
                push_redraw(Some(old), Some(new), effects);
            }
            State::Annotating { .. } => {}
        }
    }

    fn on_press(&mut self, point: Point, effects: &mut Vec<Effect>) {
        self.pointer = Some(point);
        let mut next = None;
        let mut grip = None;
        match &self.state {
            State::Hovering { snap } => {
                let old = snap.hovered;
                self.snap = snap.clone();
                self.snap.clear_hover();
                push_redraw(old, None, effects);
                next = Some(State::Dragging { from: point });
            }
            State::Annotating { rect, .. } => {
                if let Some(hit) = rect.grip_at(point) {
                    grip = Some(hit);
                    next = Some(State::Adjusting {
                        rect: *rect,
                        grip: hit,
                    });
                } else if rect.contains(point) {
                    grip = Some(Grip::Body);
                    next = Some(State::Adjusting {
                        rect: *rect,
                        grip: Grip::Body,
                    });
                }
            }
            State::Dragging { .. } | State::Adjusting { .. } => {}
        }
        if let Some(grip) = grip {
            self.active_grip = grip;
        }
        if let Some(next) = next {
            self.state = next;
        }
    }

    fn on_release(&mut self, point: Point, effects: &mut Vec<Effect>) {
        let previous = self.pointer.replace(point);
        let mut next = None;
        let mut finish = None;
        let mut redraw = None;
        match &self.state {
            State::Dragging { from } => {
                let from = *from;
                let old = Rect::from_points(from, previous.unwrap_or(point));
                let dragged = Rect::from_points(from, point);
                let rect = if dragged.width.0 <= 3 && dragged.height.0 <= 3 {
                    self.snap.hover_at(from).unwrap_or(dragged)
                } else {
                    dragged
                };
                if rect.is_empty() {
                    redraw = Some((Some(old), None));
                    next = Some(self.resting_state());
                } else if self.annotate {
                    redraw = Some((Some(old), Some(rect)));
                    next = Some(State::Annotating {
                        rect,
                        tool: self.tool,
                    });
                } else {
                    finish = Some(Selection::new(rect));
                    next = Some(self.resting_state());
                }
            }
            State::Adjusting { rect, grip } => {
                let old = *rect;
                let new = match grip {
                    Grip::Body => rect.translated(point - previous.unwrap_or(point)),
                    Grip::NorthWest
                    | Grip::North
                    | Grip::NorthEast
                    | Grip::East
                    | Grip::SouthEast
                    | Grip::South
                    | Grip::SouthWest
                    | Grip::West => grip.resize_to(*rect, point),
                };
                if new.is_empty() {
                    redraw = Some((Some(old), None));
                    next = Some(self.resting_state());
                } else if self.annotate {
                    redraw = Some((Some(old), Some(new)));
                    next = Some(State::Annotating {
                        rect: new,
                        tool: self.tool,
                    });
                } else {
                    finish = Some(Selection::new(new));
                    next = Some(self.resting_state());
                }
            }
            State::Hovering { .. } | State::Annotating { .. } => {}
        }
        if let Some((old, new)) = redraw {
            push_redraw(old, new, effects);
        }
        if let Some(selection) = finish {
            effects.push(Effect::Finish(selection));
        }
        if let Some(next) = next {
            self.state = next;
        }
    }

    fn on_right_click(&mut self, effects: &mut Vec<Effect>) {
        let selected = self.selected_rect();
        let cancel = matches!(&self.state, State::Hovering { .. } | State::Dragging { .. });
        let reset = matches!(
            &self.state,
            State::Adjusting { .. } | State::Annotating { .. }
        );
        if cancel {
            effects.push(Effect::Cancel);
        }
        if reset {
            if let Some(rect) = selected {
                push_redraw(Some(rect), None, effects);
            }
            self.pointer = None;
        }
        if cancel || reset {
            let resting = self.resting_state();
            self.state = resting;
        }
    }

    fn cancel(&mut self, effects: &mut Vec<Effect>) {
        effects.push(Effect::Cancel);
        self.pointer = None;
        let resting = self.resting_state();
        self.state = resting;
    }

    fn on_tab(&mut self, effects: &mut Vec<Effect>) {
        if !matches!(&self.state, State::Hovering { .. }) {
            return;
        }
        self.sync_snap();
        let old = self.snap.hovered;
        self.snap.cycle_mode();
        if let Some(point) = self.pointer {
            self.snap.update_hover(point);
        }
        let new = self.snap.hovered;
        self.state = State::Hovering {
            snap: self.snap.clone(),
        };
        push_redraw(old, new, effects);
    }

    fn on_arrow(
        &mut self,
        direction: Direction,
        shift: bool,
        alt: bool,
        effects: &mut Vec<Effect>,
    ) {
        let amount = Px(if shift { 10 } else { 1 });
        let current = match &self.state {
            State::Adjusting { rect, .. } | State::Annotating { rect, .. } => Some(*rect),
            State::Hovering { .. } | State::Dragging { .. } => None,
        };
        let Some(old) = current else {
            return;
        };
        let new = if alt {
            self.active_grip.nudge(old, direction, amount)
        } else {
            old.translated(direction.delta(amount))
        };
        if new == old || new.is_empty() {
            return;
        }
        match &mut self.state {
            State::Adjusting { rect, .. } | State::Annotating { rect, .. } => *rect = new,
            State::Hovering { .. } | State::Dragging { .. } => return,
        }
        push_redraw(Some(old), Some(new), effects);
    }

    fn on_set_tool(&mut self, tool: Tool, effects: &mut Vec<Effect>) {
        self.tool = tool;
        if let State::Annotating {
            rect,
            tool: current,
        } = &mut self.state
        {
            *current = tool;
            effects.push(Effect::Redraw(*rect));
        }
    }

    fn on_commit(&mut self, effects: &mut Vec<Effect>) {
        let selected = match &self.state {
            State::Annotating { rect, .. } => Some(*rect),
            State::Hovering { .. } | State::Dragging { .. } | State::Adjusting { .. } => None,
        };
        if let Some(rect) = selected {
            effects.push(Effect::Finish(Selection::new(rect)));
            self.pointer = None;
            let resting = self.resting_state();
            self.state = resting;
        }
    }

    fn on_snap_candidates(&mut self, set: SnapSet, effects: &mut Vec<Effect>) {
        self.sync_snap();
        let old = self.snap.hovered;
        self.snap.set_candidates(set);
        if !matches!(&self.state, State::Hovering { .. }) {
            return;
        }
        if let Some(point) = self.pointer {
            self.snap.update_hover(point);
        }
        let new = self.snap.hovered;
        self.state = State::Hovering {
            snap: self.snap.clone(),
        };
        push_redraw(old, new, effects);
    }

    fn on_copy_colour(&mut self, rgb: Rgb, effects: &mut Vec<Effect>) {
        if matches!(
            &self.state,
            State::Annotating {
                tool: Tool::Colour,
                ..
            }
        ) {
            effects.push(Effect::CopyColour(rgb));
        }
    }
}

impl Default for Overlay {
    fn default() -> Self {
        Self::new()
    }
}

fn push_redraw(old: Option<Rect>, new: Option<Rect>, effects: &mut Vec<Effect>) {
    let rect = match (old, new) {
        (None, None) => return,
        (Some(old), None) => old,
        (None, Some(new)) => new,
        (Some(old), Some(new)) => {
            if old == new {
                return;
            }
            old.union(new)
        }
    };
    if !rect.is_empty() {
        effects.push(Effect::Redraw(rect));
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::geometry::{GRIPS, HANDLE_SIZE};
    use crate::overlay::snap::SnapSet;

    use super::*;

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn annotating(rect: Rect) -> Overlay {
        Overlay {
            state: State::Annotating {
                rect,
                tool: Tool::Highlighter,
            },
            ..Overlay::with_annotate(true)
        }
    }

    fn drag(overlay: Overlay, from: Point, to: Point) -> (Overlay, Vec<Effect>) {
        let (overlay, _) = overlay.on(Input::Press(from));
        let (overlay, _) = overlay.on(Input::Move(to));
        overlay.on(Input::Release(to))
    }

    #[test]
    fn starts_hovering_with_no_selection() {
        let overlay = Overlay::new();
        assert!(matches!(&overlay.state, State::Hovering { .. }));
        assert_eq!(overlay.pointer, None);
        assert_eq!(overlay.selection(), None);
        assert_eq!(overlay.active_grip, Grip::SouthEast);
        assert_eq!(overlay.tool, Tool::Highlighter);
        assert_eq!(overlay.tool.id(), "highlight");
    }

    #[test]
    fn move_sets_pointer_and_reports_new_snap_rect() {
        let overlay = Overlay::new();
        let (overlay, effects) = overlay.on(Input::SnapCandidates(SnapSet::new(
            vec![rect(10, 10, 100, 100)],
            vec![rect(20, 20, 20, 20)],
        )));
        assert!(effects.is_empty());
        let (overlay, effects) = overlay.on(Input::Move(point(50, 50)));
        assert_eq!(overlay.pointer, Some(point(50, 50)));
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(10, 10, 100, 100));
    }

    #[test]
    fn tab_cycles_snapping_and_redraws_the_hovered_rect() {
        let overlay = Overlay::new();
        let (overlay, _) = overlay.on(Input::SnapCandidates(SnapSet::new(
            vec![rect(0, 0, 500, 500)],
            vec![rect(100, 100, 50, 50)],
        )));
        let (overlay, _) = overlay.on(Input::Move(point(120, 120)));
        let (overlay, effects) = overlay.on(Input::Tab);
        assert_eq!(overlay.snap.mode, crate::overlay::SnapMode::Control);
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(0, 0, 500, 500));
        let (overlay, effects) = overlay.on(Input::Tab);
        assert_eq!(overlay.snap.mode, crate::overlay::SnapMode::Window);
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(0, 0, 500, 500));
    }

    #[test]
    fn press_starts_dragging_and_clears_hover() {
        let overlay = Overlay::new();
        let (overlay, _) = overlay.on(Input::SnapCandidates(SnapSet::new(
            vec![rect(0, 0, 100, 100)],
            vec![],
        )));
        let (overlay, _) = overlay.on(Input::Move(point(50, 50)));
        let (overlay, effects) = overlay.on(Input::Press(point(50, 50)));
        assert_eq!(
            overlay.state,
            State::Dragging {
                from: point(50, 50)
            }
        );
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(0, 0, 100, 100));
    }

    #[test]
    fn clicking_a_window_captures_the_topmost_candidate() {
        let top = rect(20, 30, 200, 150);
        let (overlay, _) = Overlay::new().on(Input::SnapCandidates(SnapSet::new(
            vec![top, rect(0, 0, 500, 500)],
            vec![],
        )));
        let (_, effects) = drag(overlay, point(60, 70), point(62, 71));
        assert!(effects.contains(&Effect::Finish(Selection::new(top))));
    }

    #[test]
    fn clicking_a_window_enters_annotation_when_enabled() {
        let target = rect(20, 30, 200, 150);
        let (overlay, _) = Overlay::with_annotate(true)
            .on(Input::SnapCandidates(SnapSet::new(vec![target], vec![])));
        let (overlay, effects) = drag(overlay, point(60, 70), point(60, 70));
        assert_eq!(overlay.selected_rect(), Some(target));
        assert!(effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Finish(_))));
    }

    #[test]
    fn dragging_over_a_window_captures_the_dragged_rectangle() {
        let (overlay, _) = Overlay::new().on(Input::SnapCandidates(SnapSet::new(
            vec![rect(0, 0, 500, 500)],
            vec![],
        )));
        let (_, effects) = drag(overlay, point(60, 70), point(80, 90));
        assert!(effects.contains(&Effect::Finish(Selection::new(rect(60, 70, 20, 20)))));
    }

    #[test]
    fn release_without_annotate_finishes_with_physical_rect() {
        let overlay = Overlay::new();
        let (overlay, effects) = drag(overlay, point(10, 20), point(110, 220));
        assert_eq!(
            overlay.state,
            State::Hovering {
                snap: SnapState::new()
            }
        );
        let [Effect::Finish(selection)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(selection.rect, rect(10, 20, 100, 200));
        assert_eq!(selection.origin(), point(10, 20));
        assert_eq!(selection.size(), Size::new(Px(100), Px(200)));
    }

    #[test]
    fn degenerate_drags_do_not_finish_or_panic() {
        for (from, to) in [
            (point(10, 10), point(10, 10)),
            (point(10, 10), point(10, 100)),
            (point(10, 10), point(100, 10)),
        ] {
            let (overlay, effects) = drag(Overlay::new(), from, to);
            assert!(matches!(&overlay.state, State::Hovering { .. }));
            assert!(effects
                .iter()
                .all(|effect| !matches!(effect, Effect::Finish(_))));
        }
    }

    #[test]
    fn release_with_annotate_enters_annotating_with_current_tool() {
        let overlay = Overlay::with_annotate(true);
        let (overlay, effects) = drag(overlay, point(10, 10), point(60, 80));
        assert_eq!(
            overlay.state,
            State::Annotating {
                rect: rect(10, 10, 50, 70),
                tool: Tool::Highlighter
            }
        );
        assert!(effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Finish(_))));
    }

    #[test]
    fn press_inside_selection_moves_it() {
        let (overlay, _) = annotating(rect(100, 100, 200, 200)).on(Input::Press(point(150, 150)));
        assert_eq!(
            overlay.state,
            State::Adjusting {
                rect: rect(100, 100, 200, 200),
                grip: Grip::Body
            }
        );
        let (overlay, _) = overlay.on(Input::Move(point(160, 170)));
        assert_eq!(overlay.selected_rect(), Some(rect(110, 120, 200, 200)));
        let (overlay, effects) = overlay.on(Input::Release(point(160, 170)));
        assert!(matches!(&overlay.state, State::Annotating { .. }));
        assert!(effects
            .iter()
            .all(|effect| !matches!(effect, Effect::Finish(_))));
    }

    #[test]
    fn every_handle_starts_the_matching_adjustment() {
        let base = rect(100, 100, 300, 300);
        for grip in GRIPS {
            let (overlay, _) = annotating(base).on(Input::Press(grip.point(base)));
            assert_eq!(overlay.active_grip, grip);
            assert_eq!(overlay.state, State::Adjusting { rect: base, grip });
        }
    }

    #[test]
    fn every_edge_and_corner_resizes_by_dragging() {
        let base = rect(100, 100, 300, 300);
        for grip in GRIPS {
            let target = match grip {
                Grip::NorthWest => point(50, 60),
                Grip::North => point(200, 60),
                Grip::NorthEast => point(450, 60),
                Grip::East => point(450, 200),
                Grip::SouthEast => point(450, 460),
                Grip::South => point(200, 460),
                Grip::SouthWest => point(50, 460),
                Grip::West => point(50, 200),
                Grip::Body => point(0, 0),
            };
            let (overlay, _) = annotating(base).on(Input::Press(grip.point(base)));
            let (overlay, _) = overlay.on(Input::Move(target));
            let (overlay, _) = overlay.on(Input::Release(target));
            let expected = grip.resize_to(base, target);
            assert_eq!(overlay.selected_rect(), Some(expected));
            assert_eq!(overlay.active_grip, grip);
        }
    }

    #[test]
    fn adjusting_handles_are_nine_pixels() {
        assert_eq!(HANDLE_SIZE, Px(9));
        let base = rect(100, 100, 300, 300);
        let near = Grip::NorthWest.point(base).offset(Px(4), Px(4));
        let (overlay, _) = annotating(base).on(Input::Press(near));
        assert!(matches!(
            overlay.state,
            State::Adjusting {
                grip: Grip::NorthWest,
                ..
            }
        ));
    }

    #[test]
    fn arrow_keys_move_one_pixel_and_shift_moves_ten() {
        let (overlay, effects) = annotating(rect(100, 100, 100, 100)).on(Input::Arrow {
            direction: Direction::Right,
            shift: false,
            alt: false,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(101, 100, 100, 100)));
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(100, 100, 101, 100));
        let (overlay, _) = overlay.on(Input::Arrow {
            direction: Direction::Down,
            shift: true,
            alt: false,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(101, 110, 100, 100)));
        let (overlay, _) = overlay.on(Input::Arrow {
            direction: Direction::Left,
            shift: false,
            alt: false,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(100, 110, 100, 100)));
        let (overlay, _) = overlay.on(Input::Arrow {
            direction: Direction::Up,
            shift: true,
            alt: false,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(100, 100, 100, 100)));
    }

    #[test]
    fn alt_arrow_resizes_from_the_active_grip() {
        let base = rect(100, 100, 200, 200);
        let (overlay, _) = annotating(base).on(Input::Press(Grip::SouthEast.point(base)));
        let (overlay, _) = overlay.on(Input::Arrow {
            direction: Direction::Right,
            shift: false,
            alt: true,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(100, 100, 201, 200)));
        let (overlay, _) = overlay.on(Input::Arrow {
            direction: Direction::Down,
            shift: true,
            alt: true,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(100, 100, 201, 210)));
        let (overlay, _) = annotating(base).on(Input::Press(Grip::NorthWest.point(base)));
        let (overlay, _) = overlay.on(Input::Arrow {
            direction: Direction::Up,
            shift: true,
            alt: true,
        });
        assert_eq!(overlay.selected_rect(), Some(rect(100, 90, 200, 210)));
    }

    #[test]
    fn right_click_before_selection_cancels() {
        let (overlay, effects) = Overlay::new().on(Input::RightClick);
        assert!(matches!(&overlay.state, State::Hovering { .. }));
        assert_eq!(effects, vec![Effect::Cancel]);
        let (overlay, _) = Overlay::new().on(Input::Press(point(10, 10)));
        let (overlay, effects) = overlay.on(Input::RightClick);
        assert!(matches!(&overlay.state, State::Hovering { .. }));
        assert_eq!(effects, vec![Effect::Cancel]);
    }

    #[test]
    fn right_click_with_selection_resets_to_hovering() {
        let (overlay, effects) = annotating(rect(10, 10, 100, 100)).on(Input::RightClick);
        assert_eq!(
            overlay.state,
            State::Hovering {
                snap: SnapState::new()
            }
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(&effects[0], Effect::Redraw(_)));
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::Cancel)));
    }

    #[test]
    fn escape_and_cancel_always_cancel() {
        let (_, effects) = Overlay::new().on(Input::Escape);
        assert_eq!(effects, vec![Effect::Cancel]);
        let (_, effects) = annotating(rect(10, 10, 100, 100)).on(Input::Escape);
        assert_eq!(effects, vec![Effect::Cancel]);
        let (_, effects) = annotating(rect(10, 10, 100, 100)).on(Input::Cancel);
        assert_eq!(effects, vec![Effect::Cancel]);
        let (overlay, _) = Overlay::new().on(Input::Press(point(10, 10)));
        let (overlay, effects) = overlay.on(Input::Escape);
        assert_eq!(effects, vec![Effect::Cancel]);
        assert!(matches!(&overlay.state, State::Hovering { .. }));
    }

    #[test]
    fn toolbar_tool_changes_persist() {
        let (overlay, effects) = annotating(rect(10, 10, 100, 100)).on(Input::SetTool(Tool::Rect));
        assert_eq!(overlay.tool, Tool::Rect);
        assert_eq!(
            overlay.state,
            State::Annotating {
                rect: rect(10, 10, 100, 100),
                tool: Tool::Rect
            }
        );
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(10, 10, 100, 100));
    }

    #[test]
    fn copy_save_and_upload_finish_from_annotating() {
        for input in [Input::Copy, Input::Save, Input::Upload] {
            let (overlay, effects) = annotating(rect(5, 6, 70, 80)).on(input);
            let [Effect::Finish(selection)] = effects.as_slice() else {
                panic!()
            };
            assert_eq!(selection.rect, rect(5, 6, 70, 80));
            assert!(matches!(&overlay.state, State::Hovering { .. }));
        }
    }

    #[test]
    fn copy_and_save_do_nothing_before_annotating() {
        for input in [Input::Copy, Input::Save, Input::Upload] {
            let (_, effects) = Overlay::new().on(input);
            assert!(effects.is_empty());
        }
    }

    #[test]
    fn copy_colour_only_works_with_the_colour_tool() {
        let rgb = Rgb::new(1, 2, 3);
        let (_, effects) = annotating(rect(10, 10, 10, 10)).on(Input::CopyColour(rgb));
        assert!(effects.is_empty());
        let (overlay, _) = annotating(rect(10, 10, 10, 10)).on(Input::SetTool(Tool::Colour));
        let (overlay, effects) = overlay.on(Input::CopyColour(rgb));
        assert_eq!(effects, vec![Effect::CopyColour(rgb)]);
        assert!(matches!(&overlay.state, State::Annotating { .. }));
    }

    #[test]
    fn late_snap_candidates_update_hover_in_hovering_only() {
        let (overlay, _) = Overlay::new().on(Input::Move(point(50, 50)));
        let (overlay, effects) = overlay.on(Input::SnapCandidates(SnapSet::new(
            vec![rect(0, 0, 100, 100)],
            vec![],
        )));
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(0, 0, 100, 100));
        let (overlay, effects) = overlay.on(Input::SnapCandidates(SnapSet::new(
            vec![rect(0, 0, 10, 10)],
            vec![],
        )));
        let [Effect::Redraw(area)] = effects.as_slice() else {
            panic!()
        };
        assert_eq!(*area, rect(0, 0, 100, 100));
        assert_eq!(overlay.snap.hovered, None);
    }

    #[test]
    fn tab_and_arrows_are_ignored_outside_hovering_and_selection() {
        let (overlay, effects) = Overlay::new().on(Input::Tab);
        assert!(effects.is_empty());
        assert!(matches!(&overlay.state, State::Hovering { .. }));
        let (overlay, effects) = Overlay::new().on(Input::Arrow {
            direction: Direction::Left,
            shift: false,
            alt: false,
        });
        assert!(effects.is_empty());
        assert!(matches!(&overlay.state, State::Hovering { .. }));
    }

    proptest! {
        #[test]
        fn random_drag_sequences_never_panic(
            ax in -5000i32..5000,
            ay in -5000i32..5000,
            bx in -5000i32..5000,
            by in -5000i32..5000,
            annotate in any::<bool>(),
        ) {
            let from = point(ax, ay);
            let to = point(bx, by);
            let (overlay, _) = Overlay::with_annotate(annotate).on(Input::Press(from));
            let (overlay, _) = overlay.on(Input::Move(to));
            let (overlay, effects) = overlay.on(Input::Release(to));
            for effect in effects {
                if let Effect::Finish(selection) = effect {
                    prop_assert!(selection.rect.width.0 >= 0);
                    prop_assert!(selection.rect.height.0 >= 0);
                }
            }
            let (_, _) = overlay.on(Input::Escape);
        }

        #[test]
        fn random_handle_drags_never_panic(
            x in -2000i32..2000,
            y in -2000i32..2000,
            w in 1i32..2000,
            h in 1i32..2000,
            px in -3000i32..3000,
            py in -3000i32..3000,
            index in 0usize..8,
        ) {
            let base = rect(x, y, w, h);
            let grip = GRIPS[index];
            let (overlay, _) = annotating(base).on(Input::Press(grip.point(base)));
            let (overlay, _) = overlay.on(Input::Move(point(px, py)));
            let (overlay, effects) = overlay.on(Input::Release(point(px, py)));
            for effect in effects {
                if let Effect::Redraw(area) = effect {
                    prop_assert!(area.width.0 >= 0);
                    prop_assert!(area.height.0 >= 0);
                }
            }
            let _ = overlay.on(Input::Escape);
        }
    }
}
