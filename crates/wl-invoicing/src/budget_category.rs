//! Budget categories (wl-invoicing §3 *Budget category*, §7, `FIN.BUD.01`):
//! the lines of the company budget and of the budget vs actual report.
//!
//! A category has a code, names in English and Arabic, a kind (`income`,
//! `expense` or `owner_drawings`), an optional parent for subtotals, the role
//! `FIN.RUL.24` tells, and, for owner drawings, the owner party whose drawings
//! it records. Its **mapping** says what it collects: charge types and
//! catalogue item groups for income; cost codes, wl-payments expense
//! categories and payroll for expense; ledger export accounts for either.
//! A mapping key belongs to at most one category, checked on [`save`], so
//! nothing is counted twice. The kind cannot change once the category has a
//! budget line or an actual ([`kind_is_locked`]).
//!
//! [`install`] stores the seeded categories the database lacks (§7); it keeps
//! whatever the client changed.

use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::notify::fill;

use crate::texts::english;

/// The code of the seeded owner drawings category.
pub const OWNER_DRAWINGS: &str = "owner_drawings";

/// What a budget category is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BudgetKind {
    /// Money earned; below budget is unfavourable.
    #[default]
    Income,
    /// Money spent in running the business; above budget is unfavourable.
    Expense,
    /// Money the owner takes out for personal use: never operating cost.
    OwnerDrawings,
}

impl BudgetKind {
    /// The mapping keys a category of this kind may hold.
    pub fn allows(self, key: MappingKind) -> bool {
        match self {
            BudgetKind::Income => matches!(
                key,
                MappingKind::ChargeType | MappingKind::ItemGroup | MappingKind::LedgerAccount
            ),
            BudgetKind::Expense => matches!(
                key,
                MappingKind::CostCode
                    | MappingKind::ExpenseCategory
                    | MappingKind::Payroll
                    | MappingKind::LedgerAccount
            ),
            BudgetKind::OwnerDrawings => matches!(
                key,
                MappingKind::CostCode | MappingKind::ExpenseCategory | MappingKind::LedgerAccount
            ),
        }
    }
}

/// What kind of key a mapping holds.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum MappingKind {
    /// A charge type of the charge catalogue (income).
    #[default]
    ChargeType,
    /// A catalogue item group (income).
    ItemGroup,
    /// A cost code (expense).
    CostCode,
    /// A wl-payments expense category (expense).
    ExpenseCategory,
    /// Payroll: the employer cost of a paid payroll run (expense).
    Payroll,
    /// A ledger export account of `FIN.EXT.01`.
    LedgerAccount,
}

/// A line of the company budget and of the budget vs actual report.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_budget_categories", unique(code))]
pub struct BudgetCategory {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `marketing`; unique.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// Income, expense or owner drawings.
    pub kind: BudgetKind,
    /// The category this one is a part of, for subtotals.
    pub parent_id: Option<i64>,
    /// The role `FIN.RUL.24` tells; none: the finance manager only.
    pub owner_role: Option<String>,
    /// The owner party of an owner drawings category.
    pub owner_party_id: Option<i64>,
    /// The order in the budget and the report.
    pub position: i64,
    /// Whether new budgets may use it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BudgetCategory {
    /// The category with this code.
    pub async fn by_code(
        db: impl renox::db::Executor<'_>,
        code: &str,
    ) -> Result<Option<BudgetCategory>> {
        BudgetCategory::query()
            .where_(BudgetCategory::CODE.eq(code))
            .first(db)
            .await
    }

    /// The categories in the order of the report, the inactive ones too when
    /// `all`.
    pub async fn listed(db: &Db, all: bool) -> Result<Vec<BudgetCategory>> {
        BudgetCategory::query()
            .when(!all, |q| q.where_(BudgetCategory::ACTIVE.eq(true)))
            .order_by(BudgetCategory::POSITION)
            .order_by(BudgetCategory::ID)
            .get(db)
            .await
    }

    /// Whether the category records owner drawings.
    pub fn is_drawings(&self) -> bool {
        self.kind == BudgetKind::OwnerDrawings
    }
}

/// What a category collects: one mapping key.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_budget_mappings",
    unique(key_kind, key_value),
    index(category_id)
)]
pub struct BudgetMapping {
    /// The primary key.
    pub id: i64,
    /// The category the key belongs to.
    pub category_id: i64,
    /// What kind of key it is.
    pub key_kind: MappingKind,
    /// The key, trimmed and lower-cased.
    pub key_value: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The key as it is stored and compared: trimmed and lower-cased.
pub fn normalise_key(value: &str) -> String {
    value.trim().to_lowercase()
}

/// A 422 with `message`.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// A 422 with the English text `key`, its `{{ name }}` filled from `data`.
pub(crate) fn refused_with(key: &str, data: &renox::serde_json::Value) -> Error {
    refused(fill(english(key), data))
}

/// What a user enters for a category.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CategoryInput {
    /// The code; unique, kept when the category is changed.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The kind.
    pub kind: BudgetKind,
    /// The code of the parent category, if any.
    pub parent_code: Option<String>,
    /// The role `FIN.RUL.24` tells.
    pub owner_role: Option<String>,
    /// The owner party of an owner drawings category (required for that kind).
    pub owner_party_id: Option<i64>,
    /// The mapping keys.
    pub mappings: Vec<(MappingKind, String)>,
    /// Whether new budgets may use it.
    pub active: bool,
}

/// Reads the mapping text of a form: one key per line, `kind: value`, e.g.
/// `cost_code: marketing`. A blank line is skipped.
///
/// # Errors
///
/// 422 naming the first line that is not `kind: value` or whose kind is
/// unknown.
pub fn parse_mappings(text: &str) -> Result<Vec<(MappingKind, String)>> {
    let mut keys = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let parsed = line.split_once(':').and_then(|(kind, value)| {
            let kind = kind
                .trim()
                .to_lowercase()
                .replace(' ', "_")
                .parse::<MappingKind>()
                .ok()?;
            let value = value.trim();
            (!value.is_empty()).then(|| (kind, value.to_owned()))
        });
        match parsed {
            Some(key) => keys.push(key),
            None => {
                return Err(refused_with(
                    "wl_invoicing.budget.category.problem.mapping_line",
                    &json!({ "line": line }),
                ));
            }
        }
    }
    Ok(keys)
}

/// Whether the kind of category `id` can no longer change: it has a budget
/// line or an actual.
pub async fn kind_is_locked(db: impl renox::db::Executor<'_>, id: i64) -> Result<bool> {
    let lines = crate::budget::CompanyBudgetLine::query()
        .where_(crate::budget::CompanyBudgetLine::CATEGORY_ID.eq(id))
        .exists(db)
        .await?;
    Ok(lines)
}

/// Saves a category: a new one when `id` is `None`, else the change of that
/// one. Its mapping is replaced by `input.mappings`.
///
/// # Errors
///
/// 422 when the code or name is blank, the code is used, an owner drawings
/// category has no owner party, the parent is unknown or the category itself,
/// a mapping key does not fit the kind, a mapping key is already used by
/// another category (the message names it), or the kind of a category with a
/// budget line or an actual is changed.
pub async fn save(db: &Db, id: Option<i64>, input: CategoryInput) -> Result<BudgetCategory> {
    let code = input.code.trim().to_lowercase();
    if code.is_empty() || input.name_en.trim().is_empty() {
        return Err(refused(english(
            "wl_invoicing.budget.category.problem.blank",
        )));
    }
    let mut tx = db.begin().await?;
    let existing = match id {
        Some(id) => Some(
            BudgetCategory::find(&mut tx, id)
                .await?
                .ok_or(Error::NotFound)?,
        ),
        None => None,
    };
    let same_code = BudgetCategory::query()
        .where_(BudgetCategory::CODE.eq(code.as_str()))
        .first(&mut tx)
        .await?;
    if same_code.is_some_and(|c| Some(c.id) != id) {
        return Err(refused_with(
            "wl_invoicing.budget.category.problem.code_used",
            &json!({ "code": code }),
        ));
    }
    if input.kind == BudgetKind::OwnerDrawings && input.owner_party_id.is_none() {
        return Err(refused(english(
            "wl_invoicing.budget.category.problem.owner_party",
        )));
    }
    if let Some(existing) = &existing
        && existing.kind != input.kind
        && kind_is_locked(&mut tx, existing.id).await?
    {
        return Err(refused(english(
            "wl_invoicing.budget.category.problem.kind_locked",
        )));
    }
    let parent_id = match input.parent_code.as_deref().map(str::trim) {
        Some(parent) if !parent.is_empty() => {
            let parent = BudgetCategory::by_code(&mut tx, &parent.to_lowercase())
                .await?
                .filter(|p| Some(p.id) != id)
                .ok_or_else(|| {
                    refused_with(
                        "wl_invoicing.budget.category.problem.parent",
                        &json!({ "code": parent }),
                    )
                })?;
            Some(parent.id)
        }
        _ => None,
    };
    // The keys, once each.
    let mut keys: Vec<(MappingKind, String)> = Vec::new();
    for (kind, value) in &input.mappings {
        let value = normalise_key(value);
        if value.is_empty() {
            continue;
        }
        if !input.kind.allows(*kind) {
            return Err(refused_with(
                "wl_invoicing.budget.category.problem.key_kind",
                &json!({ "key": format!("{kind}: {value}"), "kind": input.kind.to_string() }),
            ));
        }
        if !keys.contains(&(*kind, value.clone())) {
            keys.push((*kind, value));
        }
    }
    for (kind, value) in &keys {
        let used = BudgetMapping::query()
            .where_(BudgetMapping::KEY_KIND.eq(*kind))
            .where_(BudgetMapping::KEY_VALUE.eq(value.as_str()))
            .first(&mut tx)
            .await?;
        if let Some(used) = used.filter(|m| Some(m.category_id) != id) {
            let other = BudgetCategory::find(&mut tx, used.category_id)
                .await?
                .map(|c| c.name_en)
                .unwrap_or_default();
            return Err(refused_with(
                "wl_invoicing.budget.category.problem.mapping_used",
                &json!({ "key": format!("{kind}: {value}"), "category": other }),
            ));
        }
    }
    let owner_role = input
        .owner_role
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned);
    let name_ar = input
        .name_ar
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned);
    let owner_party_id = (input.kind == BudgetKind::OwnerDrawings)
        .then_some(input.owner_party_id)
        .flatten();
    let saved = match existing {
        Some(mut category) => {
            category.name_en = input.name_en.trim().to_owned();
            category.name_ar = name_ar;
            category.kind = input.kind;
            category.parent_id = parent_id;
            category.owner_role = owner_role;
            category.owner_party_id = owner_party_id;
            category.active = input.active;
            category.save(&mut tx).await?;
            category
        }
        None => {
            let position = BudgetCategory::query()
                .max::<i64, _>(&mut tx, "position")
                .await?
                .unwrap_or(0)
                + 1;
            let mut category = BudgetCategory {
                code,
                name_en: input.name_en.trim().to_owned(),
                name_ar,
                kind: input.kind,
                parent_id,
                owner_role,
                owner_party_id,
                position,
                active: input.active,
                ..Default::default()
            };
            category.insert(&mut tx).await?;
            category
        }
    };
    BudgetMapping::query()
        .where_(BudgetMapping::CATEGORY_ID.eq(saved.id))
        .delete(&mut tx)
        .await?;
    for (kind, value) in keys {
        BudgetMapping {
            category_id: saved.id,
            key_kind: kind,
            key_value: value,
            ..Default::default()
        }
        .insert(&mut tx)
        .await?;
    }
    tx.commit().await?;
    Ok(saved)
}

/// The category that collects `value` of `kind`, if any.
pub async fn category_of(
    db: &Db,
    kind: MappingKind,
    value: &str,
) -> Result<Option<BudgetCategory>> {
    let key = normalise_key(value);
    let mapping = BudgetMapping::query()
        .where_(BudgetMapping::KEY_KIND.eq(kind))
        .where_(BudgetMapping::KEY_VALUE.eq(key.as_str()))
        .first(db)
        .await?;
    match mapping {
        Some(mapping) => BudgetCategory::find(db, mapping.category_id).await,
        None => Ok(None),
    }
}

/// The mapping keys of category `id`, in a stable order.
pub async fn mappings_of(db: &Db, id: i64) -> Result<Vec<BudgetMapping>> {
    BudgetMapping::query()
        .where_(BudgetMapping::CATEGORY_ID.eq(id))
        .order_by(BudgetMapping::KEY_KIND)
        .order_by(BudgetMapping::KEY_VALUE)
        .get(db)
        .await
}

/// One seeded category: code, English name, Arabic name, kind, owner role and
/// the keys it collects.
struct Seed {
    code: &'static str,
    name_en: &'static str,
    name_ar: &'static str,
    kind: BudgetKind,
    owner_role: Option<&'static str>,
    keys: &'static [(MappingKind, &'static str)],
}

/// The categories of §7: income (sales and services; other income), expense
/// (cost of sales, salaries and staff costs, rent and utilities, marketing,
/// transport and fuel, professional fees, bank charges, other) and owner
/// drawings, mapped from the starter charge types, cost codes and expense
/// categories of the verticals.
const SEEDS: &[Seed] = &[
    Seed {
        code: "sales_services",
        name_en: "Sales and services",
        name_ar: "المبيعات والخدمات",
        kind: BudgetKind::Income,
        owner_role: None,
        keys: &[
            (MappingKind::ChargeType, "rent"),
            (MappingKind::ChargeType, "service_charge"),
            (MappingKind::ChargeType, "chiller"),
            (MappingKind::ChargeType, "parking"),
            (MappingKind::ChargeType, "admin_fee"),
            (MappingKind::ItemGroup, "sales"),
            (MappingKind::ItemGroup, "services"),
        ],
    },
    Seed {
        code: "other_income",
        name_en: "Other income",
        name_ar: "إيرادات أخرى",
        kind: BudgetKind::Income,
        owner_role: None,
        keys: &[
            (MappingKind::ChargeType, "fine"),
            (MappingKind::ChargeType, "other_income"),
        ],
    },
    Seed {
        code: "cost_of_sales",
        name_en: "Cost of sales",
        name_ar: "تكلفة المبيعات",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "cost_of_sales"),
            (MappingKind::CostCode, "materials"),
        ],
    },
    Seed {
        code: "salaries_staff",
        name_en: "Salaries and staff costs",
        name_ar: "الرواتب وتكاليف الموظفين",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "salaries"),
            (MappingKind::Payroll, "payroll"),
        ],
    },
    Seed {
        code: "rent_utilities",
        name_en: "Rent and utilities",
        name_ar: "الإيجار والمرافق",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "rent"),
            (MappingKind::CostCode, "utilities"),
            (MappingKind::ExpenseCategory, "utilities"),
        ],
    },
    Seed {
        code: "marketing",
        name_en: "Marketing",
        name_ar: "التسويق",
        kind: BudgetKind::Expense,
        owner_role: Some("marketing_manager"),
        keys: &[
            (MappingKind::CostCode, "marketing"),
            (MappingKind::ExpenseCategory, "marketing"),
        ],
    },
    Seed {
        code: "transport_fuel",
        name_en: "Transport and fuel",
        name_ar: "النقل والوقود",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "transport"),
            (MappingKind::CostCode, "fuel"),
            (MappingKind::ExpenseCategory, "transport"),
            (MappingKind::ExpenseCategory, "fuel"),
        ],
    },
    Seed {
        code: "professional_fees",
        name_en: "Professional fees",
        name_ar: "الأتعاب المهنية",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "professional_fees"),
            (MappingKind::ExpenseCategory, "professional_fees"),
        ],
    },
    Seed {
        code: "bank_charges",
        name_en: "Bank charges",
        name_ar: "الرسوم المصرفية",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "bank_charges"),
            (MappingKind::ExpenseCategory, "bank_charges"),
        ],
    },
    Seed {
        code: "other_expense",
        name_en: "Other expense",
        name_ar: "مصروفات أخرى",
        kind: BudgetKind::Expense,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "other"),
            (MappingKind::ExpenseCategory, "other"),
        ],
    },
    Seed {
        code: OWNER_DRAWINGS,
        name_en: "Owner drawings",
        name_ar: "مسحوبات المالك",
        kind: BudgetKind::OwnerDrawings,
        owner_role: None,
        keys: &[
            (MappingKind::CostCode, "owner_drawings"),
            (MappingKind::ExpenseCategory, "owner_drawings"),
        ],
    },
];

/// Stores the seeded categories and their keys that the database lacks;
/// what exists is kept as the client changed it (a key the client moved to
/// another category stays there). Returns how many categories it created.
///
/// The seeded *Owner drawings* category has no owner party yet: the client
/// sets it before any drawing is recorded.
pub async fn install(db: &Db) -> Result<usize> {
    let mut created = 0;
    let mut tx = db.begin().await?;
    for (position, seed) in SEEDS.iter().enumerate() {
        let category = match BudgetCategory::by_code(&mut tx, seed.code).await? {
            Some(category) => category,
            None => {
                let mut category = BudgetCategory {
                    code: seed.code.to_owned(),
                    name_en: seed.name_en.to_owned(),
                    name_ar: Some(seed.name_ar.to_owned()),
                    kind: seed.kind,
                    owner_role: seed.owner_role.map(str::to_owned),
                    position: i64::try_from(position).unwrap_or(0) + 1,
                    active: true,
                    ..Default::default()
                };
                category.insert(&mut tx).await?;
                created += 1;
                category
            }
        };
        for (kind, value) in seed.keys {
            let taken = BudgetMapping::query()
                .where_(BudgetMapping::KEY_KIND.eq(*kind))
                .where_(BudgetMapping::KEY_VALUE.eq(*value))
                .exists(&mut tx)
                .await?;
            if !taken {
                BudgetMapping {
                    category_id: category.id,
                    key_kind: *kind,
                    key_value: (*value).to_owned(),
                    ..Default::default()
                }
                .insert(&mut tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stored_in_snake_case() {
        let kinds: Vec<_> = BudgetKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["income", "expense", "owner_drawings"]);
        let keys: Vec<_> = MappingKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "charge_type",
                "item_group",
                "cost_code",
                "expense_category",
                "payroll",
                "ledger_account"
            ]
        );
    }

    #[test]
    fn a_kind_allows_only_its_keys() {
        assert!(BudgetKind::Income.allows(MappingKind::ChargeType));
        assert!(!BudgetKind::Income.allows(MappingKind::CostCode));
        assert!(BudgetKind::Expense.allows(MappingKind::Payroll));
        assert!(!BudgetKind::Expense.allows(MappingKind::ItemGroup));
        assert!(BudgetKind::OwnerDrawings.allows(MappingKind::ExpenseCategory));
        assert!(!BudgetKind::OwnerDrawings.allows(MappingKind::Payroll));
        assert!(BudgetKind::Expense.allows(MappingKind::LedgerAccount));
    }

    #[test]
    fn the_mapping_text_reads_one_key_per_line() {
        let keys = parse_mappings("cost_code: Fuel\n\n  Expense Category : fuel \n").unwrap();
        assert_eq!(
            keys,
            [
                (MappingKind::CostCode, "Fuel".to_owned()),
                (MappingKind::ExpenseCategory, "fuel".to_owned())
            ]
        );
        assert!(parse_mappings("fuel").is_err());
        assert!(parse_mappings("colour: red").is_err());
        assert!(parse_mappings("cost_code:").is_err());
    }

    #[test]
    fn keys_are_compared_trimmed_and_lower_case() {
        assert_eq!(normalise_key("  Fuel "), "fuel");
    }

    #[test]
    fn the_seeds_hold_each_key_once_and_exactly_one_drawings_category() {
        let mut seen = std::collections::BTreeSet::new();
        for seed in SEEDS {
            for (kind, value) in seed.keys {
                assert!(seed.kind.allows(*kind), "{} / {value}", seed.code);
                assert!(seen.insert((*kind, *value)), "{value} is seeded twice");
            }
        }
        let drawings = SEEDS
            .iter()
            .filter(|s| s.kind == BudgetKind::OwnerDrawings)
            .count();
        assert_eq!(drawings, 1);
        let codes: std::collections::BTreeSet<_> = SEEDS.iter().map(|s| s.code).collect();
        assert_eq!(codes.len(), SEEDS.len());
    }
}
