pub const WIDTH: f32 = 344.0;
pub const BORDER: f32 = 1.0;
pub const RADIUS: f32 = 8.0;
pub const FADE_MS: u32 = 120;
pub const FADE_OFFSET: f32 = 6.0;
pub const COPIED_MS: u32 = 1500;

const HEADER_HEIGHT: f32 = 62.0;
const BUTTON: f32 = 36.0;
const TILES_HEIGHT: f32 = 68.0;
const TILE_HEIGHT: f32 = 56.0;
const TILE_GAP: f32 = 6.0;
const TILE_COLUMNS: f32 = 4.0;
const SECTION_PAD_X: f32 = 16.0;
const RECENT_PAD_TOP: f32 = 10.0;
const RECENT_PAD_BOTTOM: f32 = 12.0;
const RECENT_HEADER: f32 = 24.0;
const RECENT_GAP: f32 = 8.0;
const THUMB_GAP: f32 = 8.0;
const THUMB_META_GAP: f32 = 5.0;
const THUMB_META_HEIGHT: f32 = 14.0;
const EMPTY_HEIGHT: f32 = 16.0;
const UPLOAD_ROW: f32 = 39.0;
const UPLOAD_TITLE: f32 = 16.0;
const FOOTER_HEIGHT: f32 = 49.0;
const COPY_WIDTH: f32 = 72.0;
const COPY_HEIGHT: f32 = 32.0;
const VIEW_ALL_WIDTH: f32 = 56.0;
pub const MAX_RECENT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tile {
    Region,
    Window,
    Screen,
}

impl Tile {
    pub const ALL: [Self; 3] = [Self::Region, Self::Window, Self::Screen];

    pub fn action(self) -> &'static str {
        match self {
            Self::Region => "capture_region",
            Self::Window => "capture_window",
            Self::Screen => "capture_screen",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Region => "Region",
            Self::Window => "Window",
            Self::Screen => "Screen",
        }
    }

    pub fn chord(self) -> &'static str {
        match self {
            Self::Region => "PrtSc",
            Self::Window => "Alt+PrtSc",
            Self::Screen => "Ctrl+PrtSc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Item {
    OpenMain,
    Settings,
    Tile(Tile),
    ViewAll,
    Recent(usize),
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Content {
    pub recent: usize,
    pub loaded: bool,
    pub uploads: usize,
    pub has_link: bool,
}

impl Content {
    fn recent_shown(self) -> usize {
        self.recent.min(MAX_RECENT)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub content: Content,
    pub height: f32,
    pub header: RectF,
    pub logo: RectF,
    pub title: RectF,
    pub tiles_area: RectF,
    pub recent_area: RectF,
    pub recent_title: RectF,
    pub uploads_area: Option<RectF>,
    pub footer: RectF,
    pub empty: Option<RectF>,
    items: Vec<(Item, RectF)>,
}

fn thumb_width() -> f32 {
    let inner = WIDTH - 2.0 * BORDER - 2.0 * SECTION_PAD_X;
    (inner - 2.0 * THUMB_GAP) / 3.0
}

fn thumb_height() -> f32 {
    thumb_width() * 10.0 / 16.0
}

impl Layout {
    pub fn compute(content: Content) -> Self {
        let inner_x = BORDER;
        let inner_w = WIDTH - 2.0 * BORDER;
        let mut y = BORDER;
        let mut items = Vec::new();

        let header = RectF::new(inner_x, y, inner_w, HEADER_HEIGHT);
        let settings = RectF::new(header.right() - 12.0 - BUTTON, y + 14.0, BUTTON, BUTTON);
        let open = RectF::new(settings.x - 12.0 - BUTTON, y + 14.0, BUTTON, BUTTON);
        let logo = RectF::new(inner_x + 16.0, y + 15.0, 32.0, 32.0);
        let title = RectF::new(
            logo.right() + 12.0,
            y + 14.0,
            open.x - 12.0 - (logo.right() + 12.0),
            BUTTON,
        );
        items.push((Item::OpenMain, open));
        items.push((Item::Settings, settings));
        y += HEADER_HEIGHT;

        let tiles_area = RectF::new(inner_x, y, inner_w, TILES_HEIGHT);
        let tiles_w = inner_w - 24.0;
        let tile_w = (tiles_w - TILE_GAP * (TILE_COLUMNS - 1.0)) / TILE_COLUMNS;
        for (index, tile) in Tile::ALL.iter().enumerate() {
            let x = inner_x + 12.0 + index as f32 * (tile_w + TILE_GAP);
            items.push((Item::Tile(*tile), RectF::new(x, y, tile_w, TILE_HEIGHT)));
        }
        y += TILES_HEIGHT;

        let recent_top = y;
        y += 1.0 + RECENT_PAD_TOP;
        let recent_title = RectF::new(inner_x + SECTION_PAD_X, y, 120.0, RECENT_HEADER);
        let view_all = RectF::new(
            inner_x + inner_w - SECTION_PAD_X - VIEW_ALL_WIDTH,
            y,
            VIEW_ALL_WIDTH,
            RECENT_HEADER,
        );
        items.push((Item::ViewAll, view_all));
        y += RECENT_HEADER + RECENT_GAP;
        let mut empty = None;
        if content.loaded && content.recent == 0 {
            empty = Some(RectF::new(
                inner_x + SECTION_PAD_X,
                y,
                inner_w - 2.0 * SECTION_PAD_X,
                EMPTY_HEIGHT,
            ));
            y += EMPTY_HEIGHT;
        } else {
            let th = thumb_height();
            let tw = thumb_width();
            for index in 0..content.recent_shown() {
                let x = inner_x + SECTION_PAD_X + index as f32 * (tw + THUMB_GAP);
                items.push((
                    Item::Recent(index),
                    RectF::new(x, y, tw, th + THUMB_META_GAP + THUMB_META_HEIGHT),
                ));
            }
            y += th + THUMB_META_GAP + THUMB_META_HEIGHT;
        }
        y += RECENT_PAD_BOTTOM;
        let recent_area = RectF::new(inner_x, recent_top, inner_w, y - recent_top);

        let mut uploads_area = None;
        if content.uploads > 0 {
            let top = y;
            y += 1.0
                + RECENT_PAD_TOP
                + UPLOAD_TITLE
                + RECENT_GAP
                + content.uploads as f32 * UPLOAD_ROW
                + (content.uploads as f32 - 1.0) * RECENT_GAP
                + RECENT_PAD_BOTTOM;
            uploads_area = Some(RectF::new(inner_x, top, inner_w, y - top));
        }

        let footer = RectF::new(inner_x, y, inner_w, FOOTER_HEIGHT);
        let copy = RectF::new(
            footer.right() - 8.0 - COPY_WIDTH,
            y + 1.0 + 8.0,
            COPY_WIDTH,
            COPY_HEIGHT,
        );
        items.push((Item::Copy, copy));
        y += FOOTER_HEIGHT;

        Self {
            content,
            height: y + BORDER,
            header,
            logo,
            title,
            tiles_area,
            recent_area,
            recent_title,
            uploads_area,
            footer,
            empty,
            items,
        }
    }

    pub fn rect(&self, item: Item) -> Option<RectF> {
        self.items
            .iter()
            .find(|(found, _)| *found == item)
            .map(|(_, rect)| *rect)
    }

    pub fn thumb_rect(&self, index: usize) -> Option<RectF> {
        self.rect(Item::Recent(index))
            .map(|rect| RectF::new(rect.x, rect.y, rect.w, thumb_height()))
    }

    pub fn meta_rect(&self, index: usize) -> Option<RectF> {
        self.rect(Item::Recent(index)).map(|rect| {
            RectF::new(
                rect.x,
                rect.y + thumb_height() + THUMB_META_GAP,
                rect.w,
                THUMB_META_HEIGHT,
            )
        })
    }

    pub fn is_enabled(&self, item: Item) -> bool {
        match item {
            Item::Copy => self.content.has_link,
            Item::Recent(index) => index < self.content.recent_shown(),
            _ => true,
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Item> {
        self.items
            .iter()
            .filter(|(item, _)| self.is_enabled(*item) || matches!(item, Item::Copy))
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(item, _)| *item)
    }

    pub fn focus_order(&self) -> Vec<Item> {
        let mut order = vec![
            Item::OpenMain,
            Item::Settings,
            Item::Tile(Tile::Region),
            Item::Tile(Tile::Window),
            Item::Tile(Tile::Screen),
            Item::ViewAll,
        ];
        for index in 0..self.content.recent_shown() {
            order.push(Item::Recent(index));
        }
        if self.content.has_link {
            order.push(Item::Copy);
        }
        order
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Tab,
    ShiftTab,
    Left,
    Right,
    Up,
    Down,
    Enter,
    Space,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Redraw,
    Activate(Item),
    Close,
}

#[derive(Debug, Clone, Default)]
pub struct Interaction {
    pub hover: Option<Item>,
    pub pressed: Option<Item>,
    pub focus: Option<Item>,
    pub keyboard: bool,
}

impl Interaction {
    pub fn pointer_move(&mut self, layout: &Layout, x: f32, y: f32) -> Outcome {
        let hit = layout.hit(x, y);
        self.keyboard = false;
        if hit == self.hover {
            return Outcome::Nothing;
        }
        self.hover = hit;
        Outcome::Redraw
    }

    pub fn pointer_leave(&mut self) -> Outcome {
        if self.hover.is_none() && self.pressed.is_none() {
            return Outcome::Nothing;
        }
        self.hover = None;
        self.pressed = None;
        Outcome::Redraw
    }

    pub fn pointer_down(&mut self, layout: &Layout, x: f32, y: f32) -> Outcome {
        self.keyboard = false;
        let hit = layout.hit(x, y).filter(|item| layout.is_enabled(*item));
        self.pressed = hit;
        if hit.is_some() {
            self.focus = hit;
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    pub fn pointer_up(&mut self, layout: &Layout, x: f32, y: f32) -> Outcome {
        let pressed = self.pressed.take();
        let hit = layout.hit(x, y);
        match (pressed, hit) {
            (Some(down), Some(up)) if down == up && layout.is_enabled(up) => Outcome::Activate(up),
            (Some(_), _) => Outcome::Redraw,
            _ => Outcome::Nothing,
        }
    }

    pub fn key(&mut self, layout: &Layout, key: Key) -> Outcome {
        match key {
            Key::Escape => Outcome::Close,
            Key::Enter | Key::Space => match self.focus {
                Some(item) if layout.is_enabled(item) => Outcome::Activate(item),
                _ => Outcome::Nothing,
            },
            Key::Tab | Key::Right | Key::Down => self.move_focus(layout, 1),
            Key::ShiftTab | Key::Left | Key::Up => self.move_focus(layout, -1),
        }
    }

    fn move_focus(&mut self, layout: &Layout, step: i32) -> Outcome {
        let order = layout.focus_order();
        if order.is_empty() {
            return Outcome::Nothing;
        }
        self.keyboard = true;
        let len = order.len() as i32;
        let next = match self
            .focus
            .and_then(|item| order.iter().position(|found| *found == item))
        {
            Some(position) => (position as i32 + step).rem_euclid(len),
            None if step > 0 => 0,
            None => len - 1,
        };
        self.focus = Some(order[next as usize]);
        Outcome::Redraw
    }

    pub fn show_focus_ring(&self, item: Item) -> bool {
        self.keyboard && self.focus == Some(item)
    }

    pub fn is_hot(&self, item: Item) -> bool {
        self.hover == Some(item)
    }

    pub fn is_down(&self, item: Item) -> bool {
        self.pressed == Some(item) && self.hover == Some(item)
    }
}

pub fn fade(elapsed_ms: u32, reduced_motion: bool) -> (f32, f32) {
    if reduced_motion || elapsed_ms >= FADE_MS {
        return (1.0, 0.0);
    }
    let t = elapsed_ms as f32 / FADE_MS as f32;
    let eased = 1.0 - (1.0 - t).powi(3);
    (eased, FADE_OFFSET * (1.0 - eased))
}

pub fn copy_label(copied_for_ms: Option<u32>) -> &'static str {
    match copied_for_ms {
        Some(elapsed) if elapsed < COPIED_MS => "Copied",
        _ => "Copy",
    }
}

pub fn link_without_scheme(link: &str) -> &str {
    link.strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link)
}

pub fn format_duration(milliseconds: i64) -> String {
    let total = (milliseconds as f64 / 1000.0).round() as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn relative_time(created_ms: i64, now_ms: i64, utc_offset_minutes: i64) -> String {
    let local = |ms: i64| ms + utc_offset_minutes * 60_000;
    let day = |ms: i64| local(ms).div_euclid(86_400_000);
    let created_day = day(created_ms);
    let today = day(now_ms);
    if created_day == today {
        let minutes = (now_ms - created_ms).div_euclid(60_000);
        if minutes < 60 {
            return format!("{} min ago", minutes.max(1));
        }
        return format!("{} h ago", minutes / 60);
    }
    if created_day == today - 1 {
        return "Yesterday".to_owned();
    }
    let (_, month, date) = civil_from_days(created_day);
    format!("{date} {}", MONTHS[(month - 1) as usize])
}

pub fn panel_origin(
    icon: (f32, f32, f32, f32),
    work: (f32, f32, f32, f32),
    width: f32,
    height: f32,
    margin: f32,
) -> (f32, f32) {
    let (wx, wy, ww, wh) = work;
    let min_x = wx + margin;
    let max_x = (wx + ww - margin - width).max(min_x);
    let min_y = wy + margin;
    let max_y = (wy + wh - margin - height).max(min_y);
    let icon_centre = icon.1 + icon.3 / 2.0;
    let y = if icon_centre < wy + wh / 2.0 {
        min_y
    } else {
        max_y
    };
    (max_x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(recent: usize) -> Content {
        Content {
            recent,
            loaded: true,
            uploads: 0,
            has_link: true,
        }
    }

    #[test]
    fn the_layout_is_344_wide_and_stacks_the_sections() {
        let layout = Layout::compute(content(3));
        assert!(layout.header.bottom() <= layout.tiles_area.y + 0.01);
        assert!(layout.tiles_area.bottom() <= layout.recent_area.y + 0.01);
        assert!(layout.recent_area.bottom() <= layout.footer.y + 0.01);
        assert!((layout.height - (layout.footer.bottom() + BORDER)).abs() < 0.01);
    }

    #[test]
    fn three_tiles_share_a_quarter_grid() {
        let layout = Layout::compute(content(0));
        let region = layout.rect(Item::Tile(Tile::Region)).unwrap();
        let window = layout.rect(Item::Tile(Tile::Window)).unwrap();
        let screen = layout.rect(Item::Tile(Tile::Screen)).unwrap();
        assert!((region.w - 75.0).abs() < 0.01);
        assert!((window.x - region.right() - 6.0).abs() < 0.01);
        assert!((screen.x - window.right() - 6.0).abs() < 0.01);
        assert_eq!(region.h, 56.0);
    }

    #[test]
    fn recent_thumbnails_are_sixteen_by_ten() {
        let layout = Layout::compute(content(3));
        let thumb = layout.thumb_rect(0).unwrap();
        assert!((thumb.w - 98.0).abs() < 0.01);
        assert!((thumb.h - 61.25).abs() < 0.01);
    }

    #[test]
    fn only_the_three_newest_are_shown() {
        let layout = Layout::compute(content(9));
        assert!(layout.rect(Item::Recent(2)).is_some());
        assert!(layout.rect(Item::Recent(3)).is_none());
    }

    #[test]
    fn the_empty_state_is_shorter_than_the_grid() {
        let empty = Layout::compute(content(0));
        let full = Layout::compute(content(3));
        assert!(empty.empty.is_some());
        assert!(empty.height < full.height);
        assert!(empty.rect(Item::Recent(0)).is_none());
    }

    #[test]
    fn uploads_add_a_section_per_row() {
        let none = Layout::compute(content(3));
        let one = Layout::compute(Content {
            uploads: 1,
            ..content(3)
        });
        let two = Layout::compute(Content {
            uploads: 2,
            ..content(3)
        });
        assert!(one.height > none.height);
        assert!(two.height > one.height);
        assert!(one.uploads_area.is_some());
    }

    #[test]
    fn hit_testing_finds_each_control() {
        let layout = Layout::compute(content(3));
        for item in [
            Item::OpenMain,
            Item::Settings,
            Item::Tile(Tile::Region),
            Item::Tile(Tile::Window),
            Item::Tile(Tile::Screen),
            Item::ViewAll,
            Item::Recent(0),
            Item::Recent(2),
            Item::Copy,
        ] {
            let rect = layout.rect(item).unwrap();
            assert_eq!(
                layout.hit(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0),
                Some(item)
            );
        }
        assert_eq!(layout.hit(2.0, 2.0), None);
    }

    #[test]
    fn copy_is_not_focusable_or_activatable_without_a_link() {
        let layout = Layout::compute(Content {
            has_link: false,
            ..content(1)
        });
        assert!(!layout.is_enabled(Item::Copy));
        assert!(!layout.focus_order().contains(&Item::Copy));
        let mut interaction = Interaction::default();
        let rect = layout.rect(Item::Copy).unwrap();
        interaction.pointer_down(&layout, rect.x + 5.0, rect.y + 5.0);
        assert_eq!(
            interaction.pointer_up(&layout, rect.x + 5.0, rect.y + 5.0),
            Outcome::Nothing
        );
    }

    #[test]
    fn a_click_activates_only_when_released_on_the_pressed_control() {
        let layout = Layout::compute(content(3));
        let region = layout.rect(Item::Tile(Tile::Region)).unwrap();
        let window = layout.rect(Item::Tile(Tile::Window)).unwrap();
        let mut interaction = Interaction::default();
        interaction.pointer_down(&layout, region.x + 4.0, region.y + 4.0);
        assert_eq!(
            interaction.pointer_up(&layout, region.x + 4.0, region.y + 4.0),
            Outcome::Activate(Item::Tile(Tile::Region))
        );
        interaction.pointer_down(&layout, region.x + 4.0, region.y + 4.0);
        assert_eq!(
            interaction.pointer_up(&layout, window.x + 4.0, window.y + 4.0),
            Outcome::Redraw
        );
    }

    #[test]
    fn hover_changes_request_a_redraw_once() {
        let layout = Layout::compute(content(3));
        let region = layout.rect(Item::Tile(Tile::Region)).unwrap();
        let mut interaction = Interaction::default();
        assert_eq!(
            interaction.pointer_move(&layout, region.x + 3.0, region.y + 3.0),
            Outcome::Redraw
        );
        assert_eq!(
            interaction.pointer_move(&layout, region.x + 4.0, region.y + 4.0),
            Outcome::Nothing
        );
        assert!(interaction.is_hot(Item::Tile(Tile::Region)));
        assert_eq!(interaction.pointer_leave(), Outcome::Redraw);
        assert!(!interaction.is_hot(Item::Tile(Tile::Region)));
    }

    #[test]
    fn tab_walks_the_focus_order_and_wraps() {
        let layout = Layout::compute(content(2));
        let order = layout.focus_order();
        let mut interaction = Interaction::default();
        for item in &order {
            interaction.key(&layout, Key::Tab);
            assert_eq!(interaction.focus, Some(*item));
        }
        interaction.key(&layout, Key::Tab);
        assert_eq!(interaction.focus, Some(order[0]));
        interaction.key(&layout, Key::ShiftTab);
        assert_eq!(interaction.focus, Some(*order.last().unwrap()));
        assert!(interaction.show_focus_ring(*order.last().unwrap()));
    }

    #[test]
    fn enter_and_space_activate_the_focused_control_and_escape_closes() {
        let layout = Layout::compute(content(2));
        let mut interaction = Interaction::default();
        assert_eq!(interaction.key(&layout, Key::Enter), Outcome::Nothing);
        interaction.key(&layout, Key::Tab);
        interaction.key(&layout, Key::Tab);
        interaction.key(&layout, Key::Tab);
        assert_eq!(
            interaction.key(&layout, Key::Enter),
            Outcome::Activate(Item::Tile(Tile::Region))
        );
        assert_eq!(
            interaction.key(&layout, Key::Space),
            Outcome::Activate(Item::Tile(Tile::Region))
        );
        assert_eq!(interaction.key(&layout, Key::Escape), Outcome::Close);
    }

    #[test]
    fn pointer_input_hides_the_focus_ring() {
        let layout = Layout::compute(content(2));
        let mut interaction = Interaction::default();
        interaction.key(&layout, Key::Tab);
        assert!(interaction.show_focus_ring(Item::OpenMain));
        interaction.pointer_move(&layout, 2.0, 2.0);
        assert!(!interaction.show_focus_ring(Item::OpenMain));
    }

    #[test]
    fn the_fade_runs_for_120_ms_and_is_skipped_for_reduced_motion() {
        assert_eq!(fade(0, false).0, 0.0);
        assert_eq!(fade(0, false).1, FADE_OFFSET);
        let mid = fade(60, false);
        assert!(mid.0 > 0.5 && mid.0 < 1.0);
        assert_eq!(fade(120, false), (1.0, 0.0));
        assert_eq!(fade(10, true), (1.0, 0.0));
    }

    #[test]
    fn copy_shows_copied_briefly() {
        assert_eq!(copy_label(None), "Copy");
        assert_eq!(copy_label(Some(100)), "Copied");
        assert_eq!(copy_label(Some(COPIED_MS)), "Copy");
    }

    #[test]
    fn links_lose_their_scheme() {
        assert_eq!(
            link_without_scheme("https://i.imgur.com/a.png"),
            "i.imgur.com/a.png"
        );
        assert_eq!(link_without_scheme("http://x.test/a"), "x.test/a");
        assert_eq!(link_without_scheme("x.test/a"), "x.test/a");
    }

    #[test]
    fn durations_are_minutes_and_padded_seconds() {
        assert_eq!(format_duration(42_000), "0:42");
        assert_eq!(format_duration(61_400), "1:01");
        assert_eq!(format_duration(0), "0:00");
    }

    const NOON: i64 = 1_699_876_800_000;

    #[test]
    fn relative_time_matches_the_web_panel() {
        assert_eq!(relative_time(NOON - 20_000, NOON, 0), "1 min ago");
        assert_eq!(relative_time(NOON - 5 * 60_000, NOON, 0), "5 min ago");
        assert_eq!(relative_time(NOON - 59 * 60_000, NOON, 0), "59 min ago");
        assert_eq!(relative_time(NOON - 3 * 3_600_000, NOON, 0), "3 h ago");
        assert_eq!(relative_time(NOON - 13 * 3_600_000, NOON, 0), "Yesterday");
        assert_eq!(relative_time(NOON - 3 * 86_400_000, NOON, 0), "10 Nov");
        assert_eq!(relative_time(NOON - 40 * 86_400_000, NOON, 0), "4 Oct");
    }

    #[test]
    fn relative_time_uses_the_local_day() {
        assert_eq!(relative_time(NOON - 13 * 3_600_000, NOON, 600), "13 h ago");
        assert_eq!(
            relative_time(NOON - 13 * 3_600_000, NOON, -600),
            "Yesterday"
        );
    }

    #[test]
    fn civil_dates_are_correct_across_years() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn the_panel_sits_above_a_bottom_taskbar_and_inside_the_work_area() {
        let (x, y) = panel_origin(
            (1850.0, 1040.0, 24.0, 24.0),
            (0.0, 0.0, 1920.0, 1040.0),
            344.0,
            400.0,
            12.0,
        );
        assert_eq!(x, 1564.0);
        assert_eq!(y, 628.0);
        let (x, y) = panel_origin(
            (1850.0, 4.0, 24.0, 24.0),
            (0.0, 40.0, 1920.0, 1040.0),
            344.0,
            400.0,
            12.0,
        );
        assert_eq!(x, 1564.0);
        assert_eq!(y, 52.0);
    }
}
