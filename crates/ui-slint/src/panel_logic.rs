use std::time::Duration;

pub const MAX_RECENT: usize = 3;
pub const COPIED_FOR: Duration = Duration::from_millis(1500);
const MARGIN: f64 = 12.0;
pub const REOPEN_GUARD: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl TrayRect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn centre_x(self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn centre_y(self) -> f64 {
        self.y + self.height / 2.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarEdge {
    Top,
    Bottom,
    Left,
    Right,
}

pub fn taskbar_edge(work: Bounds, monitor: Bounds) -> TaskbarEdge {
    if work.y > monitor.y {
        TaskbarEdge::Top
    } else if work.x > monitor.x {
        TaskbarEdge::Left
    } else if work.right() < monitor.right() {
        TaskbarEdge::Right
    } else {
        TaskbarEdge::Bottom
    }
}

pub fn shrink_first(current: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    (current.0.min(target.0), current.1.min(target.1))
}

pub fn placement(
    icon: TrayRect,
    work: Bounds,
    monitor: Bounds,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let minimum_x = work.x + MARGIN;
    let maximum_x = (work.right() - MARGIN - width).max(minimum_x);
    let minimum_y = work.y + MARGIN;
    let maximum_y = (work.bottom() - MARGIN - height).max(minimum_y);
    let icon_y = (icon.centre_y() - height / 2.0).clamp(minimum_y, maximum_y);
    let (x, y) = match taskbar_edge(work, monitor) {
        TaskbarEdge::Bottom => (maximum_x, maximum_y),
        TaskbarEdge::Top => (maximum_x, minimum_y),
        TaskbarEdge::Left => (minimum_x, icon_y),
        TaskbarEdge::Right => (maximum_x, icon_y),
    };
    (x.clamp(minimum_x, maximum_x), y.clamp(minimum_y, maximum_y))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Close,
    Ignore,
    Open,
}

pub fn toggle(open: bool, since_closed: Option<Duration>) -> Toggle {
    if open {
        Toggle::Close
    } else if since_closed.is_some_and(|elapsed| elapsed < REOPEN_GUARD) {
        Toggle::Ignore
    } else {
        Toggle::Open
    }
}

pub fn link_without_scheme(link: &str) -> &str {
    link.strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link)
}

#[allow(clippy::cast_possible_truncation)]
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
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
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
    let name = MONTHS
        .get(usize::try_from(month).unwrap_or(1).saturating_sub(1))
        .copied()
        .unwrap_or("");
    format!("{date} {name}")
}

pub fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{} KB", (bytes + KIB / 2) / KIB)
    } else {
        #[allow(clippy::cast_precision_loss)]
        let megabytes = bytes as f64 / MIB as f64;
        format!("{megabytes:.1} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resize_never_grows_before_the_window_has_moved() {
        assert_eq!(shrink_first((344, 228), (344, 316)), (344, 228));
        assert_eq!(shrink_first((344, 316), (344, 228)), (344, 228));
        assert_eq!(shrink_first((344, 228), (400, 200)), (344, 200));
    }

    const MONITOR: Bounds = Bounds::new(0.0, 0.0, 1920.0, 1080.0);
    const WIDTH: f64 = 344.0;
    const HEIGHT: f64 = 600.0;

    fn icon() -> TrayRect {
        TrayRect::new(1850.0, 1040.0, 24.0, 24.0)
    }

    fn middle_icon() -> TrayRect {
        TrayRect::new(48.0, 528.0, 24.0, 24.0)
    }

    #[test]
    fn bottom_taskbar_sits_above_the_tray_icon() {
        let work = Bounds::new(0.0, 0.0, 1920.0, 1040.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Bottom);
        assert_eq!(
            placement(icon(), work, MONITOR, WIDTH, HEIGHT),
            (1564.0, 428.0)
        );
    }

    #[test]
    fn top_taskbar_sits_below_the_taskbar() {
        let work = Bounds::new(0.0, 40.0, 1920.0, 1040.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Top);
        assert_eq!(
            placement(icon(), work, MONITOR, WIDTH, HEIGHT),
            (1564.0, 52.0)
        );
    }

    #[test]
    fn left_taskbar_sits_next_to_the_icon() {
        let work = Bounds::new(48.0, 0.0, 1872.0, 1080.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Left);
        assert_eq!(
            placement(middle_icon(), work, MONITOR, WIDTH, HEIGHT),
            (60.0, 240.0)
        );
    }

    #[test]
    fn right_taskbar_sits_next_to_the_icon() {
        let work = Bounds::new(0.0, 0.0, 1872.0, 1080.0);
        assert_eq!(taskbar_edge(work, MONITOR), TaskbarEdge::Right);
        assert_eq!(
            placement(middle_icon(), work, MONITOR, WIDTH, HEIGHT),
            (1516.0, 240.0)
        );
    }

    #[test]
    fn vertical_placement_stays_inside_the_work_area() {
        let work = Bounds::new(48.0, 0.0, 1872.0, 1080.0);
        let high = TrayRect::new(48.0, 4.0, 24.0, 24.0);
        assert_eq!(placement(high, work, MONITOR, WIDTH, HEIGHT).1, 12.0);
        let low = TrayRect::new(48.0, 1050.0, 24.0, 24.0);
        assert_eq!(placement(low, work, MONITOR, WIDTH, HEIGHT).1, 468.0);
    }

    #[test]
    fn a_click_closes_an_open_panel() {
        assert_eq!(toggle(true, None), Toggle::Close);
        assert_eq!(toggle(true, Some(Duration::ZERO)), Toggle::Close);
    }

    #[test]
    fn the_click_that_blurred_the_panel_does_not_reopen_it() {
        assert_eq!(
            toggle(false, Some(Duration::from_millis(40))),
            Toggle::Ignore
        );
        assert_eq!(
            toggle(false, Some(Duration::from_millis(299))),
            Toggle::Ignore
        );
        assert_eq!(
            toggle(false, Some(Duration::from_millis(300))),
            Toggle::Open
        );
        assert_eq!(toggle(false, None), Toggle::Open);
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
    fn relative_time_reads_like_the_mockup() {
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
    fn upload_sizes_are_human() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "2 KB");
        assert_eq!(format_size(5 * 1024 * 1024 + 524_288), "5.5 MB");
    }
}
