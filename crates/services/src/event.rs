use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::app;
use crate::history;
use crate::hotkeys;
use crate::recording;
use crate::settings;
use crate::tools;
use crate::uploads;
use crate::workflows;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "area", content = "event", rename_all = "camelCase")]
pub enum AppEvent {
    App(app::Event),
    Settings(settings::Event),
    Hotkeys(hotkeys::Event),
    History(history::Event),
    Upload(uploads::Event),
    Workflow(workflows::Event),
    Record(recording::Event),
    Tool(tools::Event),
}
