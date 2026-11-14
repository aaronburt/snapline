use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{HistoryCellData, HistoryRowData, HistoryState, SnaplineWindow};

type Seen = Rc<RefCell<BTreeMap<i32, bool>>>;

fn cell(id: i32, index: i32) -> HistoryCellData {
    HistoryCellData {
        id,
        name: format!("capture {id}").into(),
        meta: "".into(),
        tag: "PNG".into(),
        thumb: Default::default(),
        has_thumb: false,
        selected: false,
        index,
        day: "Today".into(),
        duration: "".into(),
        badge: "".into(),
        badge_tone: "local".into(),
        type_label: "Image".into(),
        captured: "".into(),
        size: "".into(),
        timed: "".into(),
        progress: -1.0,
        show_progress: false,
        cancellable: false,
    }
}

fn settle() {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
}

fn window(seen: &Seen) -> Option<SnaplineWindow> {
    let app = SnaplineWindow::new().ok()?;
    app.set_page(3);
    app.window().set_size(slint::PhysicalSize::new(1100, 700));
    let sink = seen.clone();
    app.global::<HistoryState>().on_thumb_view(move |id, on| {
        sink.borrow_mut().insert(id, on);
    });
    Some(app)
}

fn in_view(seen: &Seen) -> Vec<i32> {
    seen.borrow()
        .iter()
        .filter(|(_, on)| **on)
        .map(|(id, _)| *id)
        .collect()
}

#[test]
fn cells_report_only_what_is_near_the_viewport() {
    i_slint_backend_testing::init_integration_test_with_mock_time();
    grid_cells_report_only_what_is_near_the_viewport();
    list_rows_report_only_what_is_near_the_viewport();
}

fn grid_cells_report_only_what_is_near_the_viewport() {
    let seen = Seen::default();
    let app = window(&seen);
    assert!(app.is_some());
    let Some(app) = app else { return };
    let s = app.global::<HistoryState>();
    let mut days = Vec::new();
    let mut all = Vec::new();
    let mut id = 0;
    for _ in 0..3 {
        let mut cells = Vec::new();
        for index in 0..40 {
            id += 1;
            cells.push(cell(id, index));
            all.push(cell(id, index));
        }
        days.push(HistoryRowData {
            label: "Today".into(),
            count: "40".into(),
            size: 40,
            cells: ModelRc::new(VecModel::from(cells)),
        });
    }
    s.set_loaded(true);
    s.set_grid_view(true);
    s.set_rows(ModelRc::new(VecModel::from(days)));
    s.set_cells(ModelRc::new(VecModel::from(all)));
    assert!(app.show().is_ok());
    settle();
    let first = in_view(&seen);
    assert!(first.contains(&1));
    assert!(!first.contains(&60));
    assert!(first.len() < 40);

    app.set_scroll_y(1500.0);
    settle();
    let scrolled = in_view(&seen);
    assert!(!scrolled.contains(&1));
    assert!(scrolled.iter().any(|id| *id > 12));

    app.set_scroll_y(0.0);
    settle();
    assert_eq!(in_view(&seen), first);
}

fn list_rows_report_only_what_is_near_the_viewport() {
    let seen = Seen::default();
    let app = window(&seen);
    assert!(app.is_some());
    let Some(app) = app else { return };
    let s = app.global::<HistoryState>();
    let all: Vec<_> = (1..=100).map(|id| cell(id, id - 1)).collect();
    s.set_loaded(true);
    s.set_grid_view(false);
    s.set_cells(ModelRc::new(VecModel::from(all)));
    assert!(app.show().is_ok());
    settle();
    let first = in_view(&seen);
    assert!(first.contains(&1));
    assert!(!first.contains(&50));

    app.set_scroll_y(2000.0);
    settle();
    let scrolled = in_view(&seen);
    assert!(!scrolled.contains(&1));
    assert!(scrolled.contains(&30));
    assert!(!scrolled.contains(&80));
}
