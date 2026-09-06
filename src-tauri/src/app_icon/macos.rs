use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;
use tauri::AppHandle;

pub fn apply(app: &AppHandle, png: Option<&[u8]>) -> Result<(), String> {
    let png = png.map(Vec::from);
    on_main(app, move || set_dock_icon(png.as_deref()))
}

fn on_main<T, F>(app: &AppHandle, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    if MainThreadMarker::new().is_some() {
        return work();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(work());
    })
    .map_err(|error| error.to_string())?;
    rx.recv()
        .map_err(|_| "macOS Dock icon main-thread channel closed".to_string())?
}

fn set_dock_icon(png: Option<&[u8]>) -> Result<(), String> {
    let marker = MainThreadMarker::new().ok_or("macOS Dock icon requires the main thread")?;
    let application = NSApplication::sharedApplication(marker);
    match png {
        Some(bytes) => {
            let image = ns_image(bytes)?;
            unsafe { application.setApplicationIconImage(Some(&image)) };
        }
        None => unsafe { application.setApplicationIconImage(None) },
    }
    Ok(())
}

fn ns_image(png: &[u8]) -> Result<Retained<NSImage>, String> {
    let data = NSData::with_bytes(png);
    NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "tray_icon_invalid: failed to create Dock image".into())
}
