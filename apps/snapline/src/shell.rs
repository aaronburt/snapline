use std::sync::{Arc, OnceLock};

use snapline_core::ports::Notice;
use snapline_services::{AppEvent, Services};
use snapline_ui::UiHandle;

use crate::slint_ui;

#[derive(Clone)]
pub struct Shell(Arc<Inner>);

struct Inner {
    services: OnceLock<Services>,
    ui: OnceLock<Option<UiHandle>>,
}

impl Shell {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            services: OnceLock::new(),
            ui: OnceLock::new(),
        }))
    }

    pub fn set_services(&self, services: Services) {
        let _ = self.0.services.set(services);
    }

    pub fn services(&self) -> Option<Services> {
        self.0.services.get().cloned()
    }

    fn ui(&self) -> Option<UiHandle> {
        self.0
            .ui
            .get_or_init(|| {
                let services = self.services()?;
                match slint_ui::start(services) {
                    Ok(handle) => Some(handle),
                    Err(error) => {
                        tracing::error!(%error, "the Snapline window could not start");
                        None
                    }
                }
            })
            .clone()
    }

    fn ui_if_started(&self) -> Option<UiHandle> {
        self.0.ui.get().and_then(Clone::clone)
    }

    pub fn open_main(&self, route: &str) {
        let Some(ui) = self.ui() else {
            self.report("Snapline could not open its window.");
            return;
        };
        let shell = self.clone();
        ui.open(
            route,
            Arc::new(move |_route: &str| {
                tracing::error!("the Snapline window could not be shown");
                shell.report("Snapline could not open its window.");
            }),
        );
    }

    pub fn emit(&self, event: &AppEvent) {
        if let Some(ui) = self.ui_if_started() {
            ui.emit_event(event);
        }
    }

    pub fn theme_changed(&self) {
        if let Some(ui) = self.ui_if_started() {
            ui.set_light_theme(crate::theme::is_light());
        }
    }

    fn report(&self, message: &str) {
        if let Some(notifier) = self.services().and_then(|services| services.notifier()) {
            notifier.toast(Notice::new("Snapline", message));
        }
    }
}
