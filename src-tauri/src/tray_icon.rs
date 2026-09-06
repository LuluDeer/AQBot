use std::io::Read;
use std::sync::{atomic::Ordering, OnceLock};

use aqbot_core::{file_store::FileStore, types::AppSettings};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::tray::{self, TrayIconAppearance};

#[derive(Clone)]
pub(crate) struct AppliedIcon {
    pub fingerprint: String,
    pub appearance: TrayIconAppearance,
    pub png: Option<Vec<u8>>,
}

#[derive(Default)]
pub(crate) struct TrayRuntime {
    pub applied: Option<AppliedIcon>,
    pub use_as_app_icon: bool,
    pub error: Option<String>,
    pub revision: u64,
}

pub(crate) fn runtime() -> &'static Mutex<TrayRuntime> {
    static RUNTIME: OnceLock<Mutex<TrayRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(TrayRuntime::default()))
}

pub(crate) fn builtin(settings: &AppSettings) -> Result<AppliedIcon, String> {
    let appearance =
        tray::tray_icon_appearance(settings.tray_icon_style).map_err(|error| error.to_string())?;
    Ok(AppliedIcon {
        fingerprint: format!("builtin:{}", appearance.is_template),
        appearance,
        png: None,
    })
}

pub(crate) fn custom(bytes: &[u8], id: &str) -> Result<AppliedIcon, String> {
    Ok(AppliedIcon {
        fingerprint: format!("{id}:{}", FileStore::hash_bytes(bytes)),
        appearance: TrayIconAppearance {
            image: crate::tray_icon_image::tray_image(bytes)?,
            is_template: false,
        },
        png: Some(bytes.to_vec()),
    })
}

pub(crate) async fn load_custom(app: &AppHandle, id: &str) -> Result<AppliedIcon, String> {
    let state = app.state::<crate::AppState>();
    let stored = aqbot_core::repo::stored_file::get_stored_file(&state.sea_db, id)
        .await
        .map_err(|error| error.to_string())?;
    if stored.mime_type != "image/png" || !stored.storage_path.starts_with("images/") {
        return Err("tray_icon_invalid: invalid stored media type".into());
    }
    let store = FileStore::new();
    let path = store
        .validated_path(&stored.storage_path)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take((crate::tray_icon_image::MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > crate::tray_icon_image::MAX_BYTES
        || FileStore::hash_bytes(&bytes) != stored.hash
    {
        return Err("tray_icon_invalid: stored image hash or size mismatch".into());
    }
    custom(&bytes, id)
}

pub(crate) fn apply_native(
    app: &AppHandle,
    settings: &AppSettings,
    icon: &AppliedIcon,
    use_as_app_icon: bool,
) -> Result<(), String> {
    apply_tray(app, settings, icon)?;
    crate::app_icon::apply(
        app,
        use_as_app_icon.then_some(icon.png.as_deref()).flatten(),
    )
}

fn apply_tray(app: &AppHandle, settings: &AppSettings, icon: &AppliedIcon) -> Result<(), String> {
    if !settings.tray_enabled {
        return Ok(());
    }
    if tray::tray_exists(app) {
        tray::apply_tray_appearance(app, &icon.appearance)
    } else {
        tray::create_tray(app, settings, icon.appearance.clone()).map_err(|error| error.to_string())
    }
}

pub(crate) fn restore_native(
    app: &AppHandle,
    settings: &AppSettings,
    previous: Option<&AppliedIcon>,
    previous_use_as_app_icon: bool,
) -> Result<(), String> {
    let tray = match previous {
        Some(icon) => apply_tray(app, settings, icon),
        None => {
            tray::destroy_tray(app);
            Ok(())
        }
    };
    let png = previous
        .filter(|_| previous_use_as_app_icon)
        .and_then(|icon| icon.png.as_deref());
    let app_icon = crate::app_icon::apply(app, png);
    match (tray, app_icon) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(tray), Err(app_icon)) => Err(format!("{tray}; {app_icon}")),
    }
}

pub(crate) fn update_availability(app: &AppHandle) {
    app.state::<crate::AppState>()
        .tray_available
        .store(tray::tray_exists(app), Ordering::Relaxed);
}

pub(crate) async fn reconcile(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let mut runtime = runtime().lock().await;
    reconcile_locked(app, settings, &mut runtime).await
}

pub(crate) async fn reconcile_locked(
    app: &AppHandle,
    settings: &AppSettings,
    runtime: &mut TrayRuntime,
) -> Result<(), String> {
    let _file_guard = aqbot_core::repo::stored_file::lock_file_references().await;
    let operation = async {
        let state = app.state::<crate::AppState>();
        let file_id = aqbot_core::repo::tray_icon::file_id(&state.sea_db)
            .await
            .map_err(|error| error.to_string())?;
        let use_as_app_icon = aqbot_core::repo::tray_icon::use_as_app_icon(&state.sea_db)
            .await
            .map_err(|error| error.to_string())?;
        let desired = match file_id {
            Some(id) => load_custom(app, &id).await?,
            None => builtin(settings)?,
        };
        if !settings.tray_enabled && tray::tray_exists(app) {
            tray::destroy_tray(app);
            crate::window_lifecycle::restore_main_window(app);
        }
        let tray_unchanged = runtime.error.is_none()
            && (!settings.tray_enabled
                || (tray::tray_exists(app)
                    && runtime
                        .applied
                        .as_ref()
                        .is_some_and(|old| old.fingerprint == desired.fingerprint)));
        let app_unchanged = runtime.use_as_app_icon == use_as_app_icon
            && runtime.applied.as_ref().map(|icon| icon.png.as_deref())
                == Some(desired.png.as_deref());
        if tray_unchanged && app_unchanged {
            if settings.tray_enabled {
                tray::request_tray_menu_sync(app);
            }
            runtime.applied = Some(desired);
            runtime.use_as_app_icon = use_as_app_icon;
            return Ok(());
        }
        if let Err(error) = apply_native(app, settings, &desired, use_as_app_icon) {
            let rollback = restore_native(
                app,
                settings,
                runtime.applied.as_ref(),
                runtime.use_as_app_icon,
            );
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback) => format!("{error}; native rollback failed: {rollback}"),
            });
        }
        runtime.applied = Some(desired);
        runtime.use_as_app_icon = use_as_app_icon;
        if settings.tray_enabled {
            tray::request_tray_menu_sync(app);
        }
        Ok(())
    }
    .await;
    runtime.error = operation.as_ref().err().cloned();
    runtime.revision += 1;
    update_availability(app);
    operation
}
