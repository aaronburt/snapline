use std::ffi::c_void;
use std::mem::size_of;

use snapline_core::image::Image;
use snapline_core::ports::{CaptureError, FrozenDesktop, FrozenMonitor, Monitor};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC,
    HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, HICON, ICONINFO,
};

use super::CaptureOptions;

pub fn freeze_all(options: CaptureOptions) -> Result<FrozenDesktop, CaptureError> {
    let monitors = crate::monitors::enumerate();
    if monitors.is_empty() {
        return Err(CaptureError::Unavailable(
            "No monitors were found.".to_string(),
        ));
    }
    let screen = ScreenDc::acquire()?;
    let mut frozen = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let image = capture_monitor(&screen, &monitor, options)?;
        frozen.push(FrozenMonitor { monitor, image });
    }
    Ok(FrozenDesktop { monitors: frozen })
}

pub(crate) fn draw_cursor(image: &mut Image, origin: (i32, i32)) -> bool {
    let Ok(width) = i32::try_from(image.width()) else {
        return false;
    };
    let Ok(height) = i32::try_from(image.height()) else {
        return false;
    };
    let Ok(dib) = DibSection::new(None, width, height) else {
        return false;
    };
    dib.write_from(image.pixels(), image.stride());
    if !draw_cursor_to_dc(dib.dc(), origin) {
        return false;
    }
    let pixels = dib.copy_bits(image.stride());
    if pixels.len() != image.pixels().len() {
        return false;
    }
    image.pixels_mut().copy_from_slice(&pixels);
    true
}

fn capture_monitor(
    screen: &ScreenDc,
    monitor: &Monitor,
    options: CaptureOptions,
) -> Result<Image, CaptureError> {
    let width = monitor.bounds.width.0;
    let height = monitor.bounds.height.0;
    if width <= 0 || height <= 0 {
        return Err(CaptureError::Failed(
            "The monitor has no drawable area.".to_string(),
        ));
    }
    let dib = DibSection::new(Some(screen.dc()), width, height)?;
    let rop = windows::Win32::Graphics::Gdi::ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
    // SAFETY: both device contexts are live, the destination bitmap is selected into the memory DC, and the source coordinates come from the monitor rectangle.
    unsafe {
        BitBlt(
            dib.dc(),
            0,
            0,
            width,
            height,
            Some(screen.dc()),
            monitor.bounds.x.0,
            monitor.bounds.y.0,
            rop,
        )
    }
    .map_err(|error| CaptureError::Failed(format!("The screen could not be copied: {error}")))?;
    if options.include_cursor {
        draw_cursor_to_dc(dib.dc(), (monitor.bounds.x.0, monitor.bounds.y.0));
    }
    let stride = usize::try_from(width)
        .map_err(|_| CaptureError::Failed("The monitor size is not supported.".to_string()))?
        * 4;
    let pixels = dib.copy_bits(stride);
    Image::new(
        u32::try_from(width)
            .map_err(|_| CaptureError::Failed("The monitor width is not supported.".to_string()))?,
        u32::try_from(height).map_err(|_| {
            CaptureError::Failed("The monitor height is not supported.".to_string())
        })?,
        stride,
        pixels,
    )
    .map_err(|error| CaptureError::Failed(error.to_string()))
}

fn draw_cursor_to_dc(dc: HDC, origin: (i32, i32)) -> bool {
    let mut info = CURSORINFO {
        cbSize: u32::try_from(size_of::<CURSORINFO>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: `info` is a live CURSORINFO with its size field set before the call.
    if unsafe { GetCursorInfo(&mut info) }.is_err() {
        return false;
    }
    if info.flags.0 & CURSOR_SHOWING.0 == 0 || info.hCursor.0.is_null() {
        return false;
    }
    let icon = HICON(info.hCursor.0);
    let mut icon_info = ICONINFO::default();
    let mut hotspot = (0, 0);
    // SAFETY: `icon` is a live cursor handle reported by GetCursorInfo and `icon_info` is a live out-parameter.
    if unsafe { GetIconInfo(icon, &mut icon_info) }.is_ok() {
        hotspot = (icon_info.xHotspot as i32, icon_info.yHotspot as i32);
        if !icon_info.hbmMask.is_invalid() {
            // SAFETY: GetIconInfo created this bitmap and it is owned by the caller.
            let _ = unsafe { DeleteObject(icon_info.hbmMask.into()) };
        }
        if !icon_info.hbmColor.is_invalid() {
            // SAFETY: GetIconInfo created this bitmap and it is owned by the caller.
            let _ = unsafe { DeleteObject(icon_info.hbmColor.into()) };
        }
    }
    let x = info.ptScreenPos.x - hotspot.0 - origin.0;
    let y = info.ptScreenPos.y - hotspot.1 - origin.1;
    // SAFETY: the device context is live, the icon handle is live, and the coordinates are relative to that device context.
    unsafe { DrawIconEx(dc, x, y, icon, 0, 0, 0, None, DI_NORMAL) }.is_ok()
}

struct ScreenDc(HDC);

impl ScreenDc {
    fn acquire() -> Result<Self, CaptureError> {
        // SAFETY: a null window handle asks for the screen device context, which is owned by this guard until ReleaseDC.
        let dc = unsafe { GetDC(None) };
        if dc.is_invalid() {
            return Err(CaptureError::Unavailable(
                "The screen device context could not be opened.".to_string(),
            ));
        }
        Ok(Self(dc))
    }

    fn dc(&self) -> HDC {
        self.0
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: this balances the successful GetDC(None) in acquire and runs once on the same thread.
        let _ = unsafe { ReleaseDC(None, self.0) };
    }
}

struct DibSection {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: usize,
    height: usize,
}

impl DibSection {
    fn new(reference: Option<HDC>, width: i32, height: i32) -> Result<Self, CaptureError> {
        if width <= 0 || height <= 0 {
            return Err(CaptureError::Failed(
                "The image size is not supported.".to_string(),
            ));
        }
        // SAFETY: a compatible memory device context accepts an optional reference device context and is released in Drop.
        let dc = unsafe { CreateCompatibleDC(reference) };
        if dc.is_invalid() {
            return Err(CaptureError::Unavailable(
                "A capture device context could not be created.".to_string(),
            ));
        }
        let info = bitmap_info(width, height);
        let mut bits: *mut c_void = std::ptr::null_mut();
        // SAFETY: `info` describes a valid 32-bit top-down bitmap and `bits` receives the section pointer.
        let bitmap = match unsafe {
            CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                // SAFETY: the device context was created above and is not selected into anything.
                let _ = unsafe { DeleteDC(dc) };
                return Err(CaptureError::Failed(format!(
                    "A capture bitmap could not be created: {error}"
                )));
            }
        };
        if bitmap.is_invalid() || bits.is_null() {
            // SAFETY: the bitmap came from CreateDIBSection and is not selected into the device context.
            let _ = unsafe { DeleteObject(bitmap.into()) };
            // SAFETY: the device context was created above and is not selected into anything.
            let _ = unsafe { DeleteDC(dc) };
            return Err(CaptureError::Failed(
                "A capture bitmap could not be created.".to_string(),
            ));
        }
        // SAFETY: the bitmap and device context are live and the returned previous object is restored in Drop.
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        Ok(Self {
            dc,
            bitmap,
            previous,
            bits: bits.cast(),
            width: width as usize,
            height: height as usize,
        })
    }

    fn dc(&self) -> HDC {
        self.dc
    }

    fn copy_bits(&self, stride: usize) -> Vec<u8> {
        let row = self.width * 4;
        let mut pixels = vec![0_u8; stride.saturating_mul(self.height)];
        if row == 0 || self.bits.is_null() {
            return pixels;
        }
        // SAFETY: the DIB section owns `row * height` readable bytes and `bits` is valid for the guard's lifetime.
        let source = unsafe { std::slice::from_raw_parts(self.bits, row * self.height) };
        for y in 0..self.height {
            let from = y * row;
            let to = y * stride;
            if to + row <= pixels.len() {
                pixels[to..to + row].copy_from_slice(&source[from..from + row]);
            }
        }
        pixels
    }

    fn write_from(&self, pixels: &[u8], stride: usize) {
        let row = self.width * 4;
        if row == 0 || self.bits.is_null() {
            return;
        }
        // SAFETY: the DIB section owns `row * height` writable bytes and `bits` is valid for the guard's lifetime.
        let destination = unsafe { std::slice::from_raw_parts_mut(self.bits, row * self.height) };
        for y in 0..self.height {
            let from = y * stride;
            let to = y * row;
            if from + row <= pixels.len() {
                destination[to..to + row].copy_from_slice(&pixels[from..from + row]);
            }
        }
    }
}

impl Drop for DibSection {
    fn drop(&mut self) {
        // SAFETY: the bitmap is selected into this device context, so the previous object is restored before deletion.
        let _ = unsafe { SelectObject(self.dc, self.previous) };
        // SAFETY: the bitmap was created by CreateDIBSection, is no longer selected, and is deleted exactly once.
        let _ = unsafe { DeleteObject(self.bitmap.into()) };
        // SAFETY: the device context was created by CreateCompatibleDC and is deleted exactly once.
        let _ = unsafe { DeleteDC(self.dc) };
    }
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: (width as u32)
            .saturating_mul(height as u32)
            .saturating_mul(4),
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "desktop"]
    fn gdi_freeze_returns_one_image_per_monitor_at_physical_size() {
        let monitors = crate::monitors::enumerate();
        let desktop = freeze_all(CaptureOptions::default()).expect("gdi freeze");
        assert_eq!(desktop.monitors.len(), monitors.len());
        for frozen in &desktop.monitors {
            assert_eq!(
                frozen.image.width(),
                u32::try_from(frozen.monitor.bounds.width.0).expect("width")
            );
            assert_eq!(
                frozen.image.height(),
                u32::try_from(frozen.monitor.bounds.height.0).expect("height")
            );
        }
    }

    #[test]
    #[ignore = "desktop"]
    fn gdi_cursor_option_keeps_the_image_size() {
        let with_cursor = freeze_all(CaptureOptions {
            include_cursor: true,
        })
        .expect("gdi freeze with cursor");
        let without_cursor = freeze_all(CaptureOptions::default()).expect("gdi freeze");
        assert_eq!(with_cursor.monitors.len(), without_cursor.monitors.len());
        for (cursor, plain) in with_cursor
            .monitors
            .iter()
            .zip(without_cursor.monitors.iter())
        {
            assert_eq!(cursor.image.width(), plain.image.width());
            assert_eq!(cursor.image.height(), plain.image.height());
            assert_eq!(cursor.image.stride(), plain.image.stride());
        }
    }
}
