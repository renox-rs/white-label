//! The charge types the vertical bills that the client configures (01 Property
//! §9, §6 *Charge type*).
//!
//! The client's charge catalogue belongs to wl-invoicing; until it keeps the
//! catalogue itself, the vertical holds the entries it needs a VAT treatment
//! for: [`EARLY_TERMINATION_FEE`], whose VAT treatment is `⚠ open:
//! accountant (A-06)`. There is no platform default: a lease cannot carry the
//! penalty of an early termination until the client has configured the charge
//! type here ([`configure`]).

use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::VatCategory;

use crate::building::refused;
use crate::texts::english;

/// The code of the early termination fee: the penalty a tenant who ends the
/// lease early owes by the contract.
pub const EARLY_TERMINATION_FEE: &str = "early_termination_fee";

/// An entry of the client's charge catalogue.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_charge_types", unique(code))]
pub struct ChargeType {
    /// The primary key.
    pub id: i64,
    /// The code charges carry (`early_termination_fee`).
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The VAT treatment of the lines billed with this type.
    pub vat_category: VatCategory,
    /// Whether new charges may use it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChargeType {
    /// The active charge type `code`, if the client configured it.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn active_by_code<'c, E: Executor<'c>>(
        db: E,
        code: &str,
    ) -> Result<Option<ChargeType>> {
        ChargeType::query()
            .where_(ChargeType::CODE.eq(code))
            .where_(ChargeType::ACTIVE.eq(true))
            .first(db)
            .await
    }
}

/// Configures (or changes) the charge type `code` with its English name and
/// its VAT treatment. Returns the stored entry.
///
/// # Errors
///
/// 422 for a blank code or name; the database's errors.
pub async fn configure(
    db: &Db,
    code: &str,
    name_en: &str,
    vat_category: VatCategory,
) -> Result<ChargeType> {
    let code = code.trim().to_lowercase();
    let name = name_en.trim();
    if code.is_empty() || name.is_empty() {
        return Err(refused(english("property.charge_types.required", &[])));
    }
    if let Some(mut found) = ChargeType::query()
        .where_(ChargeType::CODE.eq(code.as_str()))
        .first(db)
        .await?
    {
        found.name_en = name.to_owned();
        found.vat_category = vat_category;
        found.active = true;
        found.save(db).await?;
        return Ok(found);
    }
    ChargeType::create(
        db,
        ChargeType {
            code,
            name_en: name.to_owned(),
            vat_category,
            active: true,
            ..Default::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_early_termination_fee_has_a_fixed_code() {
        assert_eq!(EARLY_TERMINATION_FEE, "early_termination_fee");
    }
}
