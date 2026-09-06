use tauri::{AppHandle, Manager, WebviewWindow};

pub fn apply_window(app: &AppHandle, window: &WebviewWindow, png: &[u8]) -> Result<(), String> {
    let _ = app;
    window
        .set_icon(crate::tray_icon_image::app_icon_image(png)?)
        .map_err(|error| error.to_string())
}

pub fn restore_window(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let icon = app
        .default_window_icon()
        .ok_or_else(|| "default window icon is unavailable".to_string())?
        .clone();
    window.set_icon(icon).map_err(|error| error.to_string())
}
