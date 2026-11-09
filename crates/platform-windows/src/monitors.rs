use std::collections::HashSet;
use std::mem::size_of;

use snapline_core::geometry::{DpiScale, Px, Rect};
use snapline_core::ports::Monitor;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

const MONITOR_PRIMARY_FLAG: u32 = 1;
const DEFAULT_DPI: u32 = 96;

pub fn enumerate() -> Vec<Monitor> {
    let handles = enumerate_handles();
    let mut monitors = Vec::with_capacity(handles.len());
    for (index, handle) in handles.into_iter().enumerate() {
        if let Some(monitor) = describe(handle, index) {
            monitors.push(monitor);
        }
    }
    assign_ids(&mut monitors);
    monitors
}

pub fn primary() -> Option<Monitor> {
    enumerate().into_iter().find(|monitor| monitor.primary)
}

pub fn stable_id(name: &str) -> u32 {
    let mut hash = 2_166_136_261_u32;
    for unit in name.encode_utf16() {
        for byte in unit.to_le_bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(16_777_619);
        }
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

fn enumerate_handles() -> Vec<HMONITOR> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    let data = LPARAM(std::ptr::from_mut(&mut handles) as isize);
    // SAFETY: the callback receives a pointer to `handles`, which is live and exclusively borrowed for the duration of the call.
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(monitor_callback), data) };
    handles
}

unsafe extern "system" fn monitor_callback(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the live pointer to the caller's Vec<HMONITOR>, valid for the whole EnumDisplayMonitors call.
    let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    BOOL(1)
}

fn describe(handle: HMONITOR, index: usize) -> Option<Monitor> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>()).ok()?;
    // SAFETY: `info` is a live MONITORINFOEXW whose embedded MONITORINFO starts with a valid size field.
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        return None;
    }
    let name = device_name(&info.szDevice);
    let name = if name.is_empty() {
        format!("Display {}", index + 1)
    } else {
        name
    };
    Some(Monitor {
        id: 0,
        name,
        bounds: rect_from(info.monitorInfo.rcMonitor),
        work_area: rect_from(info.monitorInfo.rcWork),
        dpi: dpi_scale(handle),
        primary: info.monitorInfo.dwFlags & MONITOR_PRIMARY_FLAG != 0,
    })
}

fn dpi_scale(handle: HMONITOR) -> DpiScale {
    let mut x = DEFAULT_DPI;
    let mut y = DEFAULT_DPI;
    // SAFETY: both out-pointers refer to live u32 values and the monitor handle came from enumeration.
    match unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x > 0 => DpiScale::from_dpi(x),
        _ => DpiScale::ONE,
    }
}

fn rect_from(rect: RECT) -> Rect {
    Rect::new(
        Px(rect.left),
        Px(rect.top),
        Px(rect.right - rect.left),
        Px(rect.bottom - rect.top),
    )
}

fn device_name(device: &[u16; 32]) -> String {
    let length = device
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(device.len());
    String::from_utf16_lossy(&device[..length])
}

fn assign_ids(monitors: &mut [Monitor]) {
    let mut used = HashSet::new();
    for monitor in monitors {
        let base = stable_id(&monitor.name);
        let mut id = base;
        while !used.insert(id) {
            id = id.wrapping_add(1);
            if id == 0 {
                id = 1;
            }
        }
        monitor.id = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn enumeration_returns_at_least_one_monitor_at_physical_size() {
        let monitors = enumerate();
        assert!(!monitors.is_empty());
        for monitor in &monitors {
            assert!(monitor.bounds.width.0 > 0);
            assert!(monitor.bounds.height.0 > 0);
            assert!(monitor.work_area.width.0 > 0);
            assert!(monitor.work_area.height.0 > 0);
            assert!(monitor.dpi.scale() > 0.0);
            assert!(!monitor.name.is_empty());
        }
        assert!(monitors.iter().any(|monitor| monitor.primary));
    }

    #[test]
    #[ignore = "desktop"]
    fn monitor_ids_are_unique_and_stable() {
        let first = enumerate();
        let second = enumerate();
        let ids: HashSet<u32> = first.iter().map(|monitor| monitor.id).collect();
        assert_eq!(ids.len(), first.len());
        assert_eq!(
            first.iter().map(|monitor| monitor.id).collect::<Vec<_>>(),
            second.iter().map(|monitor| monitor.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn stable_id_is_deterministic() {
        assert_eq!(stable_id("\\\\.\\DISPLAY1"), stable_id("\\\\.\\DISPLAY1"));
        assert_ne!(stable_id("\\\\.\\DISPLAY1"), stable_id("\\\\.\\DISPLAY2"));
        assert_ne!(stable_id(""), 0);
    }
}
