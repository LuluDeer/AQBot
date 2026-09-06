use std::collections::HashSet;

use sea_orm::{sea_query::OnConflict, *};

use crate::entity::{settings, stored_files};
use crate::error::{AQBotError, Result};
use crate::file_store::SavedFile;

pub const FILE_ID_KEY: &str = "tray_icon_file_id";
pub const SCOPE_KEY: &str = "use_tray_icon_as_app_icon";

pub async fn file_id<C: ConnectionTrait>(db: &C) -> Result<Option<String>> {
    let row = settings::Entity::find_by_id(FILE_ID_KEY).one(db).await?;
    match row {
        None => Ok(None),
        Some(row) => serde_json::from_str(&row.value).map_err(|error| {
            AQBotError::Validation(format!("Invalid tray icon reference: {error}"))
        }),
    }
}

pub async fn use_as_app_icon<C: ConnectionTrait>(db: &C) -> Result<bool> {
    let row = settings::Entity::find_by_id(SCOPE_KEY).one(db).await?;
    match row {
        None => Ok(false),
        Some(row) => serde_json::from_str(&row.value).map_err(|error| {
            AQBotError::Validation(format!("Invalid tray icon app scope: {error}"))
        }),
    }
}

async fn upsert_setting<C: ConnectionTrait>(
    db: &C,
    key: &str,
    value: &str,
) -> std::result::Result<(), String> {
    settings::Entity::insert(settings::ActiveModel {
        key: Set(key.to_string()),
        value: Set(value.to_string()),
    })
    .on_conflict(
        OnConflict::column(settings::Column::Key)
            .update_column(settings::Column::Value)
            .to_owned(),
    )
    .exec(db)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub struct NewIcon<'a> {
    pub id: &'a str,
    pub saved: &'a SavedFile,
    pub name: &'a str,
}

/// Caller holds the file-reference lock, owns physical-file cleanup, and restores
/// its native image if this returns an error (including a failed DB commit).
pub async fn commit_change(
    db: &DatabaseConnection,
    icon: Option<NewIcon<'_>>,
    apply_native: impl FnOnce() -> std::result::Result<(), String>,
) -> std::result::Result<Vec<String>, String> {
    let txn = db.begin().await.map_err(|error| error.to_string())?;
    let operation = async {
        let previous = file_id(&txn).await.map_err(|error| error.to_string())?;
        let new_id = icon.as_ref().map(|icon| icon.id);
        if let Some(icon) = &icon {
            stored_files::ActiveModel {
                id: Set(icon.id.to_string()),
                hash: Set(icon.saved.hash.clone()),
                original_name: Set(icon.name.to_string()),
                mime_type: Set("image/png".to_string()),
                size_bytes: Set(icon.saved.size_bytes),
                storage_path: Set(icon.saved.storage_path.clone()),
                conversation_id: Set(None),
                ..Default::default()
            }
            .insert(&txn)
            .await
            .map_err(|error| error.to_string())?;
        }
        upsert_setting(
            &txn,
            FILE_ID_KEY,
            &serde_json::to_string(&new_id).map_err(|error| error.to_string())?,
        )
        .await?;
        let candidates: HashSet<String> = previous.into_iter().collect();
        let paths = super::stored_file::delete_unreferenced_candidates(&txn, &candidates)
            .await
            .map_err(|error| error.to_string())?;
        apply_native()?;
        Ok::<_, String>(paths)
    }
    .await;
    match operation {
        Ok(paths) => {
            txn.commit()
                .await
                .map_err(|error| format!("Tray icon commit failed: {error}"))?;
            Ok(paths)
        }
        Err(error) => match txn.rollback().await {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}; database rollback failed: {rollback}")),
        },
    }
}

/// Caller holds the file-reference lock and restores native icons if this errors.
pub async fn commit_scope(
    db: &DatabaseConnection,
    enabled: bool,
    apply_native: impl FnOnce() -> std::result::Result<(), String>,
) -> std::result::Result<(), String> {
    let txn = db.begin().await.map_err(|error| error.to_string())?;
    let operation = async {
        upsert_setting(
            &txn,
            SCOPE_KEY,
            &serde_json::to_string(&enabled).map_err(|error| error.to_string())?,
        )
        .await?;
        apply_native()?;
        Ok::<_, String>(())
    }
    .await;
    match operation {
        Ok(()) => {
            txn.commit()
                .await
                .map_err(|error| format!("Tray icon commit failed: {error}"))?;
            Ok(())
        }
        Err(error) => match txn.rollback().await {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}; database rollback failed: {rollback}")),
        },
    }
}

#[cfg(test)]
#[path = "tray_icon_tests.rs"]
mod tests;
