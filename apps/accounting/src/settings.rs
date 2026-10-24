//! The firm's switches (04 §7.2 rules that are off by default), kept in
//! `accounting_settings` as text. The services read them with their
//! defaults; the commands set them (BL-ACC-22: `ACC.RUL.07`).

use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A setting.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "accounting_settings", unique(key))]
pub struct AccountingSetting {
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
    Ok(AccountingSetting::query()
        .where_(AccountingSetting::KEY.eq(key))
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

/// The number in setting `key`; `default` when it was never set or is not a
/// number.
///
/// # Errors
///
/// The database's errors.
pub async fn number(db: &Db, key: &str, default: i64) -> Result<i64> {
    Ok(get(db, key)
        .await?
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(default))
}

/// Sets `key` to `value` (the caller checks who may).
///
/// # Errors
///
/// The database's errors.
pub async fn set(db: &Db, key: &str, value: &str, by: Option<i64>) -> Result {
    let stamp = now();
    let text = value.to_owned();
    let changed = AccountingSetting::query()
        .where_(AccountingSetting::KEY.eq(key))
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
        let mut row = AccountingSetting {
            key: key.to_owned(),
            value: value.to_owned(),
            updated_by: by,
            ..AccountingSetting::default()
        };
        row.save(db).await?;
    }
    Ok(())
}
