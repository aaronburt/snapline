use snapline_core::actions::ActionId;

use crate::app;
use crate::history;
use crate::hotkeys;
use crate::recording;
use crate::settings;
use crate::tools;
use crate::uploads;
use crate::workflows;

#[derive(Debug, Clone)]
pub enum Job {
    Action(ActionId),
    Area(AreaJob),
}

#[derive(Debug, Clone)]
pub enum AreaJob {
    App(app::Job),
    Settings(settings::Job),
    Hotkeys(hotkeys::Job),
    History(history::Job),
    Upload(uploads::Job),
    Workflow(workflows::Job),
    Record(recording::Job),
    Tool(tools::Job),
}
