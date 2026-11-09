use std::sync::mpsc;

use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

use crate::window::{Context, Outcome};
use crate::{EditRequest, OverlayError, OverlayResult, SelectRequest};

pub(crate) fn run_select(req: SelectRequest) -> Result<OverlayResult, OverlayError> {
    run(move || Context::build_select(req))
}

pub(crate) fn run_edit(req: EditRequest) -> Result<OverlayResult, OverlayError> {
    run(move || Context::build_edit(req))
}

fn run(
    build: impl FnOnce() -> Result<Context, OverlayError> + Send + 'static,
) -> Result<OverlayResult, OverlayError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(String::from("snapline-overlay"))
        .spawn(move || {
            let result = execute(build);
            let _ = sender.send(result);
        })
        .map_err(|_| OverlayError::Thread)?;
    receiver.recv().map_err(|_| OverlayError::Thread)?
}

fn execute(
    build: impl FnOnce() -> Result<Context, OverlayError>,
) -> Result<OverlayResult, OverlayError> {
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated overlay thread before any window exists.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut context = Box::new(build()?);
    context.activate()?;
    Context::run_message_loop();
    let outcome = context.outcome.take();
    let tool = context.scene.tool;
    let colour = context.scene.colour.rgb();
    let image = crate::window::take_flattened();
    context.destroy();
    drop(context);
    match outcome {
        Some(Outcome::Finish {
            action,
            selection,
            monitor,
        }) => Ok(OverlayResult {
            action,
            selection,
            monitor,
            tool,
            colour,
            image,
        }),
        Some(Outcome::Cancel) | None => Err(OverlayError::Cancelled),
    }
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the overlay thread with a single threaded apartment.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            initialized: result.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}
