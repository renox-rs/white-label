//! Expense categories (wl-payments §3 *Expense category*, §7): a kind of
//! spending with its VAT treatment and the budget category (wl-invoicing
//! `FIN.BUD.01`) it counts in. [`install`] stores the seeded list the
//! database lacks and keeps whatever the client changed.
//!
//! The seeded **Owner drawings** category has VAT blocked, allows no cost
//! object and maps to the budget category of kind `owner_drawings`: every
//! line coded to it is an owner drawing, approved under `FIN.ACL.21`
//! ([`crate::drawings`]). The other seeded categories are travel, fuel, meals,
//! tools and materials, parking and tolls and other, with the VAT recoverable;
//! the client blocks the ones the law does not let it recover (entertainment
//! of non-employees, vehicles available for personal use, employee personal
//! benefits ✔ [S1]). The expense claims that use the categories come with
//! their own module part.

use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The code of the seeded owner drawings category.
pub const OWNER_DRAWINGS: &str = "owner_drawings";

/// How the VAT of a line in the category is treated.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VatTreatment {
    /// The input VAT is recovered.
    #[default]
    Recoverable,
    /// The input VAT stays in the cost.
    Blocked,
    /// The spending carries no VAT.
    NoVat,
}

/// A kind of spending.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_payments_expense_categories", unique(code))]
pub struct ExpenseCategory {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `fuel`; unique.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// How the VAT of a line is treated.
    pub vat_treatment: VatTreatment,
    /// The code of the budget category it counts in, if any.
    pub budget_category: Option<String>,
    /// Whether a line may be posted to a cost object.
    pub cost_object_allowed: bool,
    /// The order in lists.
    pub position: i64,
    /// Whether new claims may use it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ExpenseCategory {
    /// The category with this code.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<ExpenseCategory>> {
        ExpenseCategory::query()
            .where_(ExpenseCategory::CODE.eq(code))
            .first(db)
            .await
    }

    /// Whether the category is owner drawings: its budget category is of kind
    /// `owner_drawings` (the seeded one is called `owner_drawings` too).
    pub fn is_owner_drawings(&self) -> bool {
        self.budget_category.as_deref() == Some(OWNER_DRAWINGS)
    }
}

/// A seeded category.
struct Seed {
    code: &'static str,
    name_en: &'static str,
    name_ar: &'static str,
    vat: VatTreatment,
    budget_category: Option<&'static str>,
    cost_object_allowed: bool,
}

/// The categories of §7, in the order of the list.
const SEEDS: &[Seed] = &[
    Seed {
        code: "travel",
        name_en: "Travel",
        name_ar: "السفر",
        vat: VatTreatment::Recoverable,
        budget_category: Some("transport_fuel"),
        cost_object_allowed: true,
    },
    Seed {
        code: "fuel",
        name_en: "Fuel",
        name_ar: "الوقود",
        vat: VatTreatment::Recoverable,
        budget_category: Some("transport_fuel"),
        cost_object_allowed: true,
    },
    Seed {
        code: "meals",
        name_en: "Meals",
        name_ar: "الوجبات",
        vat: VatTreatment::Recoverable,
        budget_category: Some("other_expense"),
        cost_object_allowed: true,
    },
    Seed {
        code: "tools_materials",
        name_en: "Tools and materials",
        name_ar: "الأدوات والمواد",
        vat: VatTreatment::Recoverable,
        budget_category: Some("cost_of_sales"),
        cost_object_allowed: true,
    },
    Seed {
        code: "parking_tolls",
        name_en: "Parking and tolls",
        name_ar: "المواقف والرسوم",
        vat: VatTreatment::Recoverable,
        budget_category: Some("transport_fuel"),
        cost_object_allowed: true,
    },
    Seed {
        code: "other",
        name_en: "Other",
        name_ar: "أخرى",
        vat: VatTreatment::Recoverable,
        budget_category: Some("other_expense"),
        cost_object_allowed: true,
    },
    Seed {
        code: OWNER_DRAWINGS,
        name_en: "Owner drawings",
        name_ar: "مسحوبات المالك",
        vat: VatTreatment::Blocked,
        budget_category: Some(OWNER_DRAWINGS),
        cost_object_allowed: false,
    },
];

/// Stores the seeded categories the database lacks; what exists is kept as
/// the client changed it. Returns how many it created.
///
/// # Errors
///
/// A database error.
pub async fn install(db: &Db) -> Result<usize> {
    let mut created = 0;
    for (position, seed) in SEEDS.iter().enumerate() {
        if ExpenseCategory::by_code(db, seed.code).await?.is_some() {
            continue;
        }
        ExpenseCategory {
            code: seed.code.to_owned(),
            name_en: seed.name_en.to_owned(),
            name_ar: Some(seed.name_ar.to_owned()),
            vat_treatment: seed.vat,
            budget_category: seed.budget_category.map(str::to_owned),
            cost_object_allowed: seed.cost_object_allowed,
            position: i64::try_from(position).unwrap_or(0) + 1,
            active: true,
            ..Default::default()
        }
        .insert(db)
        .await?;
        created += 1;
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seeds_have_unique_codes_and_owner_drawings_blocks_vat() {
        let mut codes: Vec<&str> = SEEDS.iter().map(|s| s.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), SEEDS.len());
        let drawings = SEEDS.iter().find(|s| s.code == OWNER_DRAWINGS).unwrap();
        assert_eq!(drawings.vat, VatTreatment::Blocked);
        assert!(!drawings.cost_object_allowed);
        assert_eq!(drawings.budget_category, Some(OWNER_DRAWINGS));
        assert!(
            SEEDS
                .iter()
                .filter(|s| s.code != OWNER_DRAWINGS)
                .all(|s| s.vat == VatTreatment::Recoverable && s.cost_object_allowed)
        );
    }

    #[test]
    fn a_category_is_owner_drawings_by_its_budget_category() {
        let drawings = ExpenseCategory {
            budget_category: Some(OWNER_DRAWINGS.into()),
            ..Default::default()
        };
        assert!(drawings.is_owner_drawings());
        assert!(!ExpenseCategory::default().is_owner_drawings());
    }

    #[test]
    fn vat_treatments_are_stored_in_snake_case() {
        let stored: Vec<_> = VatTreatment::ALL.iter().map(|v| v.as_str()).collect();
        assert_eq!(stored, ["recoverable", "blocked", "no_vat"]);
    }
}
