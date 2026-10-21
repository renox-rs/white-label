//! The configurable reason lists (wl-delivery §3, §7): why a visit failed
//! and why a quantity was refused. The seeder adds the §7 defaults.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The failure reason a driver uses when the recipient refuses everything.
pub const REFUSED: &str = "refused";

/// The refusal reason that needs a photo.
pub const DAMAGED: &str = "damaged";

/// Why a visit failed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_failure_reasons", unique(code))]
pub struct FailureReason {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `not_home`.
    pub code: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Whether it counts against the maximum attempts.
    pub counts: bool,
    /// Whether the delivery may be tried again after it.
    pub allows_reattempt: bool,
    /// Whether the driver may choose it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FailureReason {
    /// The active reason `code`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active_by_code(db: &Db, code: &str) -> Result<Option<FailureReason>> {
        FailureReason::query()
            .where_(FailureReason::CODE.eq(code.trim()))
            .where_(FailureReason::ACTIVE.eq(true))
            .first(db)
            .await
    }

    /// The active reasons, by name.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active_ones(db: &Db) -> Result<Vec<FailureReason>> {
        FailureReason::query()
            .where_(FailureReason::ACTIVE.eq(true))
            .order_by(FailureReason::ID)
            .get(db)
            .await
    }
}

/// Why a quantity was refused.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_refusal_reasons", unique(code))]
pub struct RefusalReason {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `damaged`.
    pub code: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Whether the driver may choose it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RefusalReason {
    /// The codes of the active reasons.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active_codes(db: &Db) -> Result<Vec<String>> {
        Ok(RefusalReason::active_ones(db)
            .await?
            .into_iter()
            .map(|reason| reason.code)
            .collect())
    }

    /// The active reasons, in order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active_ones(db: &Db) -> Result<Vec<RefusalReason>> {
        RefusalReason::query()
            .where_(RefusalReason::ACTIVE.eq(true))
            .order_by(RefusalReason::ID)
            .get(db)
            .await
    }
}

/// Why the cash collected is below the cash expected (`WRK.ACL.09`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_delivery_cod_reasons", unique(code))]
pub struct CodReason {
    /// The primary key.
    pub id: i64,
    /// The key, e.g. `recipient_short`.
    pub code: String,
    /// The English name.
    pub name_en: String,
    /// The Arabic name.
    pub name_ar: String,
    /// Who absorbs a difference with this reason unless the approver picks
    /// another: `customer`, `client`, `business` or `driver`.
    pub default_absorbed_by: String,
    /// Whether the driver may choose it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CodReason {
    /// The active reason `code`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active_by_code(db: &Db, code: &str) -> Result<Option<CodReason>> {
        CodReason::query()
            .where_(CodReason::CODE.eq(code.trim()))
            .where_(CodReason::ACTIVE.eq(true))
            .first(db)
            .await
    }

    /// The active reasons, in order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn active_ones(db: &Db) -> Result<Vec<CodReason>> {
        CodReason::query()
            .where_(CodReason::ACTIVE.eq(true))
            .order_by(CodReason::ID)
            .get(db)
            .await
    }
}
