//! The workshop's switches (08 §7.1 settings): small values the client sets at
//! onboarding, kept in `auto_settings` as text. The settings page
//! `/settings/compliance` shows them (BL-AUT-18, `host/src/app/auto_settings.rs`)
//! and the command `auto:storage-fee on|off` sets the storage fee switch; the
//! services read them with their defaults.

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The storage fee of an uncollected vehicle is charged (`AUT.RUL.18`); off
/// until the client turns it on.
pub const STORAGE_FEE_ENABLED: &str = "storage_fee_enabled";

/// A setting.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_settings", unique(key))]
pub struct Setting {
    /// The primary key.
    pub id: i64,
    /// The name.
    pub key: String,
    /// The value, as text.
    pub value: String,
    /// The user who set it.
    pub updated_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The text of setting `key`, if it was set.
///
/// # Errors
///
/// The database's errors.
pub async fn get(db: &Db, key: &str) -> Result<Option<String>> {
    Ok(Setting::query()
        .where_(Setting::KEY.eq(key))
        .first(db)
        .await?
        .map(|row| row.value))
}

/// Whether the switch `key` is on; `default` when it was never set.
///
/// # Errors
///
/// The database's errors.
pub async fn flag(db: &Db, key: &str, default: bool) -> Result<bool> {
    Ok(get(db, key)
        .await?
        .map_or(default, |text| matches!(text.as_str(), "1" | "true" | "on")))
}

/// Sets `key` to `value` (the caller checks who may).
///
/// # Errors
///
/// The database's errors.
pub async fn set(db: &Db, key: &str, value: &str, by: Option<i64>) -> Result {
    let stamp = now();
    let text = value.to_owned();
    let changed = Setting::query()
        .where_(Setting::KEY.eq(key))
        .update(
            db,
            &[
                ("value", &text),
                ("updated_by", &by),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    if changed == 0 {
        let mut row = Setting {
            key: key.to_owned(),
            value: value.to_owned(),
            updated_by: by,
            ..Setting::default()
        };
        row.save(db).await?;
    }
    Ok(())
}

/// Whether the storage fee is switched on.
///
/// # Errors
///
/// The database's errors.
pub async fn storage_fee_enabled(db: &Db) -> Result<bool> {
    flag(db, STORAGE_FEE_ENABLED, false).await
}
