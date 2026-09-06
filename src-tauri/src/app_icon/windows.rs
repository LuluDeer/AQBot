use std::sync::{Mutex, OnceLock};

use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, SendMessageW, HICON, ICONINFO, ICON_BIG, ICON_SMALL,
    WM_SETICON,
};

struct Icons {
    big: HICON,
    small: HICON,
}

unsafe impl Send for Icons {}

fn icons() -> &'static Mutex<Option<Icons>> {
    static ICONS: OnceLock<Mutex<Option<Icons>>> = OnceLock::new();
    ICONS.get_or_init(|| Mutex::new(None))
}

pub fn prepare(png: &[u8]) -> Result<(), String> {
    let rgba = crate::tray_icon_image::rasterize(png, crate::tray_icon_image::STORE_SIZE)?;
    let big = hicon_from_rgba(rgba.as_raw(), rgba.width(), rgba.height())?;
    let small = crate::tray_icon_image::rasterize(png, 32)?;
    let small = hicon_from_rgba(small.as_raw(), small.width(), small.height())?;
    replace_icons(Icons { big, small })
}

pub fn apply_window(window: &WebviewWindow) -> Result<(), String> {
    let hwnd = window_hwnd(window)?;
    let guard = icons().lock().map_err(|error| error.to_string())?;
    let current = guard
        .as_ref()
        .ok_or("Windows app icon has not been prepared")?;
    set_icons(hwnd, Some(current.small), Some(current.big));
    Ok(())
}

pub fn restore_window(window: &WebviewWindow) -> Result<(), String> {
    set_icons(window_hwnd(window)?, None, None);
    Ok(())
}

pub fn clear() -> Result<(), String> {
    replace_icons_inner(None)
}

fn replace_icons(next: Icons) -> Result<(), String> {
    replace_icons_inner(Some(next))
}

fn replace_icons_inner(next: Option<Icons>) -> Result<(), String> {
    let previous = {
        let mut guard = icons().lock().map_err(|error| error.to_string())?;
        std::mem::replace(&mut *guard, next)
    };
    destroy(previous);
    Ok(())
}

fn destroy(icons: Option<Icons>) {
    if let Some(icons) = icons {
        unsafe {
            let _ = DestroyIcon(icons.big);
            let _ = DestroyIcon(icons.small);
        }
    }
}

fn window_hwnd(window: &WebviewWindow) -> Result<HWND, String> {
    // Tauri's HWND is windows 0.61; this crate uses 0.62. Reconstruct by inner pointer.
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    Ok(HWND(hwnd.0 as _))
}

fn set_icons(hwnd: HWND, small: Option<HICON>, big: Option<HICON>) {
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            small.map(|icon| LPARAM(icon.0 as isize)),
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            big.map(|icon| LPARAM(icon.0 as isize)),
        );
    }
}

fn hicon_from_rgba(rgba: &[u8], width: u32, height: u32) -> Result<HICON, String> {
    let width = i32::try_from(width).map_err(|error| error.to_string())?;
    let height = i32::try_from(height).map_err(|error| error.to_string())?;
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return Err("GetDC failed while creating a Windows icon".into());
    }
    let color =
        unsafe { CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(|error| {
                unsafe {
                    let _ = ReleaseDC(None, screen);
                }
                error.to_string()
            })?;
    if bits.is_null() {
        unsafe {
            let _ = DeleteObject(color.into());
            let _ = ReleaseDC(None, screen);
        }
        return Err("CreateDIBSection returned a null buffer".into());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len());
    }
    let mask = unsafe { CreateBitmap(width, height, 1, 1, None) };
    if mask.is_invalid() {
        unsafe {
            let _ = DeleteObject(color.into());
            let _ = ReleaseDC(None, screen);
        }
        return Err("CreateBitmap failed while creating a Windows icon".into());
    }
    let icon_info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    let icon = unsafe { CreateIconIndirect(&icon_info) };
    unsafe {
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        let _ = ReleaseDC(None, screen);
    }
    icon.map_err(|error| error.to_string())
}
