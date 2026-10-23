//! The labour operations catalogue services (08 `AUT.JOB.03`): the menu of
//! flat-rate and hourly operations the estimates are drafted from.

use renox::prelude::*;

use crate::db::refused;
use crate::labour_line::LabourPricing;
use crate::labour_operation::LabourOperation;
use crate::permissions;
use crate::texts::english;

/// An operation to put on the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOperation {
    /// The code (`OIL-CHG`).
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// Hourly or flat-rate.
    pub pricing: LabourPricing,
    /// The standard hours of a flat-rate operation, in thousandths.
    pub hours_milli: i64,
    /// The hourly rate or the flat price, in fils.
    pub rate: i64,
}

impl NewOperation {
    /// A flat-rate operation: `price` fils for `hours_milli` standard hours.
    pub fn flat(code: &str, name_en: &str, hours_milli: i64, price: i64) -> Self {
        NewOperation {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: None,
            pricing: LabourPricing::FlatRate,
            hours_milli,
            rate: price,
        }
    }

    /// An hourly operation at `rate` fils an hour.
    pub fn hourly(code: &str, name_en: &str, rate: i64) -> Self {
        NewOperation {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: None,
            pricing: LabourPricing::Hourly,
            hours_milli: 0,
            rate,
        }
    }
}

/// Puts an operation on the menu.
///
/// # Errors
///
/// A 403 without `labour_menu.manage`; a 422 for a blank code or name, a
/// price below zero, a flat-rate operation without hours, or a code already on
/// the menu.
pub async fn create(state: &AppState, by: &AuthUser, new: NewOperation) -> Result<LabourOperation> {
    if !by.has_permission(permissions::LABOUR_MENU_MANAGE) {
        return Err(Error::Forbidden);
    }
    let code = new.code.trim().to_uppercase();
    let name = new.name_en.trim().to_owned();
    if code.is_empty() || name.is_empty() {
        return Err(refused(english("auto.labour.problem.code", &[])));
    }
    if new.rate < 0 || (new.pricing == LabourPricing::FlatRate && new.hours_milli <= 0) {
        return Err(refused(english("auto.labour.problem.price", &[])));
    }
    if LabourOperation::by_code(&state.db, &code).await?.is_some() {
        return Err(refused(english(
            "auto.labour.problem.taken",
            &[("code", &code)],
        )));
    }
    let mut operation = LabourOperation {
        code,
        name_en: name,
        name_ar: new
            .name_ar
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
        pricing: new.pricing,
        hours_milli: if new.pricing == LabourPricing::FlatRate {
            new.hours_milli
        } else {
            0
        },
        rate: new.rate,
        active: true,
        ..LabourOperation::default()
    };
    operation.save(&state.db).await?;
    Ok(operation)
}

/// Takes an operation off the menu (or puts it back); lines already drafted
/// from it keep their price.
///
/// # Errors
///
/// A 403 without `labour_menu.manage`.
pub async fn set_active(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    active: bool,
) -> Result<LabourOperation> {
    if !by.has_permission(permissions::LABOUR_MENU_MANAGE) {
        return Err(Error::Forbidden);
    }
    LabourOperation::query()
        .where_(LabourOperation::ID.eq(id))
        .update(
            &state.db,
            &[("active", &active), ("updated_at", &renox::db::now())],
        )
        .await?;
    LabourOperation::find_or_404(&state.db, id).await
}

/// The operations on the menu, by code.
///
/// # Errors
///
/// The database's errors.
pub async fn menu(db: &Db) -> Result<Vec<LabourOperation>> {
    LabourOperation::query()
        .order_by(LabourOperation::CODE)
        .get(db)
        .await
}
