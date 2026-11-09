use std::path::PathBuf;

use tauri::WebviewWindow;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, ICoreWebView2Settings3,
};
use windows::core::Interface;

pub const ADDITIONAL_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-component-update";

pub fn data_directory() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("SnapLine").join("WebView2"))
}

pub fn harden(window: &WebviewWindow) {
    if cfg!(debug_assertions) {
        return;
    }
    let result = window.with_webview(|webview| {
        if let Err(error) = apply_settings(webview.controller()) {
            tracing::warn!(%error, "could not apply WebView2 release settings");
        }
    });
    if let Err(error) = result {
        tracing::warn!(%error, "could not reach the WebView2 controller");
    }
}

fn apply_settings(controller: ICoreWebView2Controller) -> windows::core::Result<()> {
    // SAFETY: the controller belongs to the live webview for this callback.
    unsafe {
        let core = controller.CoreWebView2()?;
        let settings = core.Settings()?;
        settings.SetAreDevToolsEnabled(false)?;
        settings.SetAreDefaultContextMenusEnabled(false)?;
        settings.SetIsZoomControlEnabled(false)?;
        let settings3: ICoreWebView2Settings3 = settings.cast()?;
        settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
    }
    Ok(())
}
