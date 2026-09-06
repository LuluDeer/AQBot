use std::sync::{Mutex, OnceLock};

use serde::Serialize;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use tauri::Manager;
use tauri::{AppHandle, WebviewWindow};

use crate::conversation_popout::CONVERSATION_POPOUT_LABEL_PREFIX;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AppIconState {
    Default,
    Applied,
    #[allow(dead_code)]
    Deferred,
    #[allow(dead_code)]
    Unsupported,
}

#[derive(Clone, Default)]
struct Snapshot {
    enabled: bool,
    png: Option<Vec<u8>>,
}

fn snapshot_lock() -> &'static Mutex<Snapshot> {
    static SNAPSHOT: OnceLock<Mutex<Snapshot>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| Mutex::new(Snapshot::default()))
}

fn snapshot() -> Snapshot {
    snapshot_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

fn store_snapshot(next: Snapshot) {
    *snapshot_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = next;
}

pub fn is_icon_target_label(label: &str) -> bool {
    label == "main" || label.starts_with(CONVERSATION_POPOUT_LABEL_PREFIX)
}

pub fn linux_app_icon_unsupported(
    session_type: Option<&str>,
    wayland_display: bool,
    x11_display: bool,
) -> bool {
    match session_type {
        Some(value) if value.eq_ignore_ascii_case("wayland") => true,
        Some(value) if value.eq_ignore_ascii_case("x11") => false,
        _ => wayland_display && !x11_display,
    }
}

#[cfg(target_os = "linux")]
fn linux_unsupported() -> bool {
    linux_app_icon_unsupported(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn target_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(_, window)| is_icon_target_label(window.label()))
        .map(|(_, window)| window)
        .collect()
}

pub fn current_state(app: &AppHandle) -> AppIconState {
    let current = snapshot();
    if !current.enabled || current.png.is_none() {
        return AppIconState::Default;
    }
    #[cfg(target_os = "linux")]
    if linux_unsupported() {
        return AppIconState::Unsupported;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        return AppIconState::Applied;
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        if target_windows(app).is_empty() {
            AppIconState::Deferred
        } else {
            AppIconState::Applied
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = app;
        AppIconState::Unsupported
    }
}

pub fn apply(app: &AppHandle, png: Option<&[u8]>) -> Result<(), String> {
    store_snapshot(Snapshot {
        enabled: png.is_some(),
        png: png.map(Vec::from),
    });
    apply_now(app, png)?;
    apply_now(app, png)
}

pub fn apply_snapshot_to_window(app: &AppHandle, window: &WebviewWindow) {
    if !is_icon_target_label(window.label()) {
        return;
    }
    let current = snapshot();
    let result = match (current.enabled, current.png.as_deref()) {
        (true, Some(png)) => apply_to_window(app, window, png),
        _ => restore_window(app, window),
    };
    if let Err(error) = result {
        tracing::warn!(label = %window.label(), %error, "Failed to sync runtime app icon on a window");
    }
}

pub fn reconfirm_after_dock_visible(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let current = snapshot();
        if let Err(error) = macos::apply(
            app,
            current.enabled.then_some(current.png.as_deref()).flatten(),
        ) {
            tracing::warn!(%error, "Failed to reconfirm macOS Dock icon after becoming visible");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

fn apply_now(app: &AppHandle, png: Option<&[u8]>) -> Result<(), String> {
    match png {
        Some(bytes) => apply_custom(app, bytes),
        None => restore_all(app),
    }
}

fn apply_custom(app: &AppHandle, png: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if linux_unsupported() {
        return restore_all(app);
    }
    #[cfg(target_os = "macos")]
    {
        macos::apply(app, Some(png))?;
        return Ok(());
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        #[cfg(target_os = "windows")]
        windows::prepare(png)?;
        for window in target_windows(app) {
            apply_to_window(app, &window, png)?;
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (app, png);
        Ok(())
    }
}

fn restore_all(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return macos::apply(app, None);
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        for window in target_windows(app) {
            restore_window(app, &window)?;
        }
        #[cfg(target_os = "windows")]
        windows::clear()?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = app;
        Ok(())
    }
}

fn apply_to_window(app: &AppHandle, window: &WebviewWindow, png: &[u8]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = (app, window, png);
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        windows::apply_window(window)
    }
    #[cfg(target_os = "linux")]
    {
        if linux_unsupported() {
            return Ok(());
        }
        linux::apply_window(app, window, png)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (app, window, png);
        Ok(())
    }
}

fn restore_window(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = (app, window);
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        windows::restore_window(window)
    }
    #[cfg(target_os = "linux")]
    {
        linux::restore_window(app, window)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (app, window);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_main_and_conversation_popouts_receive_the_runtime_icon() {
        assert!(is_icon_target_label("main"));
        assert!(is_icon_target_label("conversation-popout:abc"));
        assert!(!is_icon_target_label("selection-toolbar"));
        assert!(!is_icon_target_label("screenshot-overlay"));
    }

    #[test]
    fn wayland_sessions_are_unsupported_while_x11_is_not() {
        assert!(linux_app_icon_unsupported(Some("wayland"), true, true));
        assert!(!linux_app_icon_unsupported(Some("x11"), true, true));
        assert!(linux_app_icon_unsupported(None, true, false));
        assert!(!linux_app_icon_unsupported(None, false, true));
    }
}
