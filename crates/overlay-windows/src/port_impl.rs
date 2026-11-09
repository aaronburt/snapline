use snapline_core::ports as core_ports;

use crate::{
    MagnifierSettings, MagnifierZoom, OverlayAction, OverlayError, OverlayHostImpl, SelectRequest,
    Theme,
};

impl core_ports::OverlayHost for OverlayHostImpl {
    fn select_region(
        &self,
        request: core_ports::OverlayRequest,
    ) -> Result<core_ports::OverlayResult, core_ports::OverlayError> {
        let select = SelectRequest {
            desktop: request.desktop,
            annotate: request.annotate,
            snap: request.snap,
            magnifier: MagnifierSettings {
                enabled: request.magnifier.enabled,
                zoom: match request.magnifier.zoom {
                    core_ports::MagnifierZoom::X2 => MagnifierZoom::X2,
                    core_ports::MagnifierZoom::X4 => MagnifierZoom::X4,
                    core_ports::MagnifierZoom::X8 => MagnifierZoom::X8,
                },
            },
            full_screen_crosshair: request.full_screen_crosshair,
            theme: match request.theme {
                core_ports::OverlayTheme::Dark => Theme::Dark,
                core_ports::OverlayTheme::Light => Theme::Light,
            },
        };
        let result = OverlayHostImpl::select_region(self, select).map_err(map_error)?;
        Ok(core_ports::OverlayResult {
            action: result.action.map(|action| match action {
                OverlayAction::Copy => core_ports::OverlayAction::Copy,
                OverlayAction::Save => core_ports::OverlayAction::Save,
                OverlayAction::Pin => core_ports::OverlayAction::Pin,
                OverlayAction::Upload => core_ports::OverlayAction::Upload,
            }),
            selection: result.selection,
            monitor: result.monitor,
            tool: result.tool,
            colour: result.colour,
            image: result.image,
        })
    }
}

fn map_error(error: OverlayError) -> core_ports::OverlayError {
    match error {
        OverlayError::Cancelled => core_ports::OverlayError::Cancelled,
        OverlayError::NoMonitors => core_ports::OverlayError::NoMonitors,
        OverlayError::Thread => {
            core_ports::OverlayError::Failed("The overlay thread could not be started.".to_string())
        }
        OverlayError::Device(message) => core_ports::OverlayError::Failed(message),
    }
}
