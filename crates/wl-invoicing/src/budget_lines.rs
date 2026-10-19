//! Creating a budget version and entering its lines (wl-invoicing §4
//! *Company budget* `create`, §5.15 step 1, `FIN.BUD.01`).
//!
//! [`create`] makes a `draft` for an entity and fiscal year, once per entity
//! and year at a time (a second `draft` or `submitted` version is refused):
//!
//! - when the year has an `approved` version it is a **revision**: the new
//!   version copies its lines and the lines of the months that had ended on
//!   the day of `create` (Asia/Dubai) are locked, unless the setting allows
//!   revising past months;
//! - otherwise it is a first version, which starts, as the user chooses once,
//!   empty, as a copy of last year's approved budget moved a year on (the
//!   categories inactive now are skipped), or from last year's actual per
//!   category and month.
//!
//! [`set_lines`] and [`delete_line`] change the lines of a `draft` only:
//! the category is active, the month is in the fiscal year, the amount is not
//! negative, and a locked line is never changed. [`import_csv`] fills a
//! draft from pasted CSV text with a dry run first. The platform's Excel
//! import service (`PLT.IMP.01`) is not built yet, so the rows come from CSV
//! text (`category,month,amount[,note]`); a workbook is saved as CSV for now.
//! A row with an unknown or inactive category, a month outside the year, a
//! bad amount or a repeated category and month is rejected with its row
//! number, and a file with any rejected row changes nothing.

use renox::chrono::{Months, NaiveDate};
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::LegalEntity;
use wl_kernel::documents::today;

use crate::budget::{
    BudgetStatus, CompanyBudget, CompanyBudgetLine, StartChoice, month_end, month_start,
    parse_budget_amount,
};
use crate::budget_category::{BudgetCategory, refused, refused_with};
use crate::texts::english;

/// What a user enters to create a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBudget {
    /// The legal entity.
    pub legal_entity_id: i64,
    /// The fiscal year, named by the calendar year of its first month.
    pub fiscal_year: i64,
    /// A name for the version; blank gives `2027 v1`.
    pub name: String,
    /// How a first version starts (a revision always copies).
    pub start: StartChoice,
    /// Why a revision is made; asked again at `submit`.
    pub revision_reason: Option<String>,
}

/// One line to set: category, month, amount in fils and a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineInput {
    /// The budget category.
    pub category_id: i64,
    /// Any day of the month.
    pub month: NaiveDate,
    /// The amount in AED fils, without VAT; not negative.
    pub amount: i64,
    /// A note.
    pub note: Option<String>,
}

/// The text trimmed, `None` when blank.
fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// The lines a new version starts with, before they are stored.
struct Seeded {
    choice: StartChoice,
    lines: Vec<CompanyBudgetLine>,
}

/// The lines of the approved version `approved`, copied for a revision made
/// on `today`: the months that had ended are locked unless `revise_past`.
fn revision_lines(
    lines: Vec<CompanyBudgetLine>,
    today: NaiveDate,
    revise_past: bool,
) -> Vec<CompanyBudgetLine> {
    lines
        .into_iter()
        .map(|line| CompanyBudgetLine {
            id: 0,
            budget_id: 0,
            locked: !revise_past && month_end(line.month) < today,
            created_at: None,
            updated_at: None,
            ..line
        })
        .collect()
}

/// The lines of last year's budget, a year on; categories that are inactive
/// now are skipped.
async fn last_year_lines(
    db: &mut renox::db::Transaction,
    lines: Vec<CompanyBudgetLine>,
) -> Result<Vec<CompanyBudgetLine>> {
    let mut moved = Vec::with_capacity(lines.len());
    for line in lines {
        let active = BudgetCategory::find(&mut *db, line.category_id)
            .await?
            .is_some_and(|c| c.active);
        let month = line.month.checked_add_months(Months::new(12));
        if let (true, Some(month)) = (active, month) {
            moved.push(CompanyBudgetLine {
                id: 0,
                budget_id: 0,
                month,
                locked: false,
                created_at: None,
                updated_at: None,
                ..line
            });
        }
    }
    Ok(moved)
}

/// Creates a `draft` version for the entity and year of `new`, by user `by`.
///
/// # Errors
///
/// 422 when the entity is unknown or inactive, the year is not between 2000
/// and 2100, a `draft` or `submitted` version of the entity and year exists,
/// *last year's budget* is asked for and the entity has no approved budget
/// for last year, or *last year's actual* is asked for and it cannot be read.
pub async fn create(state: &AppState, new: NewBudget, by: Option<i64>) -> Result<CompanyBudget> {
    let config = crate::config_of(state);
    if !(2000..=2100).contains(&new.fiscal_year) {
        return Err(refused(english("wl_invoicing.budget.problem.year")));
    }
    let on = today();
    let mut tx = state.db.begin().await?;
    let entity = LegalEntity::find(&mut tx, new.legal_entity_id)
        .await?
        .filter(LegalEntity::is_active)
        .ok_or_else(|| refused(english("wl_invoicing.budget.problem.entity")))?;
    let versions = CompanyBudget::versions_of(&mut tx, entity.id, new.fiscal_year).await?;
    if versions
        .iter()
        .any(|v| matches!(v.status, BudgetStatus::Draft | BudgetStatus::Submitted))
    {
        return Err(refused_with(
            "wl_invoicing.budget.problem.open_version",
            &json!({ "entity": entity.code, "year": new.fiscal_year }),
        ));
    }
    let start_month = versions
        .last()
        .map_or(config.budget_fiscal_start_month, |v| v.start_month_u32());
    let version = versions.iter().map(|v| v.version).max().unwrap_or(0) + 1;
    let approved = versions
        .iter()
        .find(|v| v.status == BudgetStatus::Approved);
    let seeded = match approved {
        Some(approved) => {
            let lines = CompanyBudgetLine::query()
                .where_(CompanyBudgetLine::BUDGET_ID.eq(approved.id))
                .order_by(CompanyBudgetLine::ID)
                .get(&mut tx)
                .await?;
            Seeded {
                choice: StartChoice::Revision,
                lines: revision_lines(lines, on, config.budget_revise_past_months),
            }
        }
        None => match new.start {
            StartChoice::Empty | StartChoice::Revision => Seeded {
                choice: StartChoice::Empty,
                lines: Vec::new(),
            },
            StartChoice::LastBudget => {
                let previous = CompanyBudget::versions_of(&mut tx, entity.id, new.fiscal_year - 1)
                    .await?
                    .into_iter()
                    .find(|v| v.status == BudgetStatus::Approved)
                    .ok_or_else(|| {
                        refused_with(
                            "wl_invoicing.budget.problem.no_last_budget",
                            &json!({ "year": new.fiscal_year - 1 }),
                        )
                    })?;
                let lines = CompanyBudgetLine::query()
                    .where_(CompanyBudgetLine::BUDGET_ID.eq(previous.id))
                    .order_by(CompanyBudgetLine::ID)
                    .get(&mut tx)
                    .await?;
                Seeded {
                    choice: StartChoice::LastBudget,
                    lines: last_year_lines(&mut tx, lines).await?,
                }
            }
            StartChoice::LastActual => {
                return Err(refused(english(
                    "wl_invoicing.budget.problem.actual_unavailable",
                )));
            }
        },
    };
    let name = clean(Some(new.name.as_str())).unwrap_or_else(|| format!("{} v{version}", new.fiscal_year));
    let mut budget = CompanyBudget {
        legal_entity_id: entity.id,
        fiscal_year: new.fiscal_year,
        start_month: i64::from(start_month),
        version,
        name,
        start_choice: seeded.choice,
        prepared_by: by,
        revision_reason: clean(new.revision_reason.as_deref()),
        ..Default::default()
    };
    budget.insert(&mut tx).await?;
    for mut line in seeded.lines {
        line.budget_id = budget.id;
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(budget)
}

/// The draft budget `id`, or why its lines cannot be changed.
async fn draft_of(
    tx: &mut renox::db::Transaction,
    id: i64,
) -> Result<CompanyBudget> {
    let budget = CompanyBudget::find(&mut *tx, id)
        .await?
        .ok_or(Error::NotFound)?;
    if budget.status != BudgetStatus::Draft {
        return Err(refused(english("wl_invoicing.budget.problem.not_editable")));
    }
    Ok(budget)
}

/// Sets the amount of each of `lines` on draft budget `budget_id`, all or
/// none.
///
/// # Errors
///
/// 422 when the budget is not a `draft`, a category is unknown or inactive,
/// a month is outside the fiscal year, an amount is negative, or a line is
/// locked.
pub async fn set_lines(db: &Db, budget_id: i64, lines: &[LineInput]) -> Result<()> {
    let mut tx = db.begin().await?;
    let budget = draft_of(&mut tx, budget_id).await?;
    let months = budget.months();
    for input in lines {
        let month = month_start(input.month);
        if !months.contains(&month) {
            return Err(refused_with(
                "wl_invoicing.budget.problem.month_outside",
                &json!({ "month": month.format("%Y-%m").to_string() }),
            ));
        }
        if input.amount < 0 {
            return Err(refused(english("wl_invoicing.budget.problem.negative")));
        }
        let category = BudgetCategory::find(&mut tx, input.category_id)
            .await?
            .filter(|c| c.active)
            .ok_or_else(|| refused(english("wl_invoicing.budget.problem.category")))?;
        let existing = CompanyBudgetLine::query()
            .where_(CompanyBudgetLine::BUDGET_ID.eq(budget.id))
            .where_(CompanyBudgetLine::CATEGORY_ID.eq(category.id))
            .where_(CompanyBudgetLine::MONTH.eq(month))
            .first(&mut tx)
            .await?;
        match existing {
            Some(line) if line.locked => {
                return Err(refused_with(
                    "wl_invoicing.budget.problem.locked",
                    &json!({
                        "category": category.name_en,
                        "month": month.format("%Y-%m").to_string(),
                    }),
                ));
            }
            Some(mut line) => {
                line.amount = input.amount;
                line.note = clean(input.note.as_deref());
                line.save(&mut tx).await?;
            }
            None => {
                CompanyBudgetLine {
                    budget_id: budget.id,
                    category_id: category.id,
                    month,
                    amount: input.amount,
                    note: clean(input.note.as_deref()),
                    locked: false,
                    ..Default::default()
                }
                .insert(&mut tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Removes the line of `category_id` in the month of `month` from draft
/// budget `budget_id`.
///
/// # Errors
///
/// 422 when the budget is not a `draft` or the line is locked.
pub async fn delete_line(
    db: &Db,
    budget_id: i64,
    category_id: i64,
    month: NaiveDate,
) -> Result<()> {
    let mut tx = db.begin().await?;
    let budget = draft_of(&mut tx, budget_id).await?;
    let month = month_start(month);
    let line = CompanyBudgetLine::query()
        .where_(CompanyBudgetLine::BUDGET_ID.eq(budget.id))
        .where_(CompanyBudgetLine::CATEGORY_ID.eq(category_id))
        .where_(CompanyBudgetLine::MONTH.eq(month))
        .first(&mut tx)
        .await?;
    if let Some(mut line) = line {
        if line.locked {
            return Err(refused_with(
                "wl_invoicing.budget.problem.locked",
                &json!({ "category": category_id.to_string(), "month": month.format("%Y-%m").to_string() }),
            ));
        }
        line.delete(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Changes the revision reason of draft budget `budget_id`.
///
/// # Errors
///
/// 422 when the budget is not a `draft`.
pub async fn set_revision_reason(db: &Db, budget_id: i64, reason: &str) -> Result<()> {
    let mut tx = db.begin().await?;
    let budget = draft_of(&mut tx, budget_id).await?;
    CompanyBudget::query()
        .where_(CompanyBudget::ID.eq(budget.id))
        .update(&mut tx, &[("revision_reason", &clean(Some(reason)))])
        .await?;
    tx.commit().await?;
    Ok(())
}

/// One rejected row of an import, with its row number in the text (the
/// header is row 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    /// The row number.
    pub row: usize,
    /// Why it is rejected.
    pub message: String,
}

/// What an import read and did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportReport {
    /// The rows read (the header and blank lines are not rows).
    pub rows: usize,
    /// The rows that are valid.
    pub valid: usize,
    /// Whether the lines were stored (never for a dry run or a file with
    /// rejected rows).
    pub applied: bool,
    /// The rejected rows.
    pub errors: Vec<RowError>,
}

/// The delimiter of `text`: the one of comma, semicolon and tab that the
/// first line has most.
fn delimiter_of(text: &str) -> char {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| first.matches(*d).count())
        .unwrap_or(',')
}

/// The cells of one CSV line; a cell may be quoted with `"`.
fn split_csv(line: &str, delimiter: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if quoted && chars.peek() == Some(&'"') {
                cell.push('"');
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if c == delimiter && !quoted {
            cells.push(cell.trim().to_owned());
            cell.clear();
        } else {
            cell.push(c);
        }
    }
    cells.push(cell.trim().to_owned());
    cells
}

/// Reads `2027-03` or `2027-03-15` as the first day of that month.
pub fn parse_month(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    let date = if text.len() == 7 {
        NaiveDate::parse_from_str(&format!("{text}-01"), "%Y-%m-%d").ok()?
    } else {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?
    };
    Some(month_start(date))
}

/// Checks pasted CSV text against draft budget `budget_id` and, unless it is
/// a dry run and when every row is valid, stores its lines. Columns:
/// `category` (the code), `month` (`2027-03`), `amount` (AED, without VAT)
/// and an optional `note`; a first line that starts with `category` is the
/// header.
///
/// # Errors
///
/// 422 when the budget is not a `draft`. Rejected rows are reported, not
/// raised.
pub async fn import_csv(
    db: &Db,
    budget_id: i64,
    text: &str,
    dry_run: bool,
) -> Result<ImportReport> {
    let budget = CompanyBudget::find_or_404(db, budget_id).await?;
    if budget.status != BudgetStatus::Draft {
        return Err(refused(english("wl_invoicing.budget.problem.not_editable")));
    }
    let months = budget.months();
    let delimiter = delimiter_of(text);
    let mut report = ImportReport::default();
    let mut inputs: Vec<(usize, LineInput, String)> = Vec::new();
    let mut seen = false;
    for (index, line) in text.lines().enumerate() {
        let row = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let cells = split_csv(line, delimiter);
        if !seen {
            seen = true;
            if cells[0].eq_ignore_ascii_case("category") {
                continue;
            }
        }
        report.rows += 1;
        let cell = |n: usize| cells.get(n).map(String::as_str).unwrap_or("");
        let mut reject = |message: String| report.errors.push(RowError { row, message });
        let code = cell(0).to_lowercase();
        let category = BudgetCategory::by_code(db, &code).await?;
        let Some(category) = category else {
            reject(fill_text("wl_invoicing.budget.import.unknown_category", &code));
            continue;
        };
        if !category.active {
            reject(fill_text("wl_invoicing.budget.import.inactive_category", &code));
            continue;
        }
        let Some(month) = parse_month(cell(1)) else {
            reject(fill_text("wl_invoicing.budget.import.bad_month", cell(1)));
            continue;
        };
        if !months.contains(&month) {
            reject(fill_text("wl_invoicing.budget.import.month_outside", cell(1)));
            continue;
        }
        let Some(amount) = parse_budget_amount(cell(2)) else {
            reject(fill_text("wl_invoicing.budget.import.bad_amount", cell(2)));
            continue;
        };
        let locked = CompanyBudgetLine::query()
            .where_(CompanyBudgetLine::BUDGET_ID.eq(budget.id))
            .where_(CompanyBudgetLine::CATEGORY_ID.eq(category.id))
            .where_(CompanyBudgetLine::MONTH.eq(month))
            .where_(CompanyBudgetLine::LOCKED.eq(true))
            .exists(db)
            .await?;
        if locked {
            reject(fill_text("wl_invoicing.budget.import.locked", cell(1)));
            continue;
        }
        let note = clean(Some(cell(3)));
        inputs.push((
            row,
            LineInput {
                category_id: category.id,
                month,
                amount,
                note,
            },
            code,
        ));
    }
    // Two rows with the same category and month are both rejected.
    let mut repeated: Vec<(usize, usize)> = Vec::new();
    for (i, (row, input, _)) in inputs.iter().enumerate() {
        for (other_row, other, _) in inputs.iter().skip(i + 1) {
            if other.category_id == input.category_id && other.month == input.month {
                repeated.push((*row, *other_row));
            }
        }
    }
    for (first, second) in &repeated {
        for row in [*first, *second] {
            if !report.errors.iter().any(|e| e.row == row) {
                report.errors.push(RowError {
                    row,
                    message: fill_pair(*first, *second),
                });
            }
        }
    }
    report.errors.sort_by_key(|e| e.row);
    let rejected: Vec<usize> = report.errors.iter().map(|e| e.row).collect();
    let valid: Vec<LineInput> = inputs
        .into_iter()
        .filter(|(row, _, _)| !rejected.contains(row))
        .map(|(_, input, _)| input)
        .collect();
    report.valid = valid.len();
    if !dry_run && report.errors.is_empty() && !valid.is_empty() {
        set_lines(db, budget.id, &valid).await?;
        report.applied = true;
    }
    Ok(report)
}

/// The English text `key` with `{{ value }}` filled.
fn fill_text(key: &str, value: &str) -> String {
    wl_kernel::notify::fill(english(key), &json!({ "value": value }))
}

/// The message of two rows that repeat a category and month.
fn fill_pair(first: usize, second: usize) -> String {
    wl_kernel::notify::fill(
        english("wl_invoicing.budget.import.repeated"),
        &json!({ "first": first, "second": second }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn line(month: NaiveDate, amount: i64) -> CompanyBudgetLine {
        CompanyBudgetLine {
            id: 9,
            budget_id: 3,
            category_id: 1,
            month,
            amount,
            ..Default::default()
        }
    }

    #[test]
    fn a_revision_locks_the_months_that_had_ended() {
        let lines = (1..=12).map(|m| line(day(2027, m, 1), 2_000_000)).collect();
        let copied = revision_lines(lines, day(2027, 4, 10), false);
        let locked: Vec<bool> = copied.iter().map(|l| l.locked).collect();
        assert_eq!(&locked[..4], [true, true, true, false]);
        assert!(locked[4..].iter().all(|l| !l));
        assert!(copied.iter().all(|l| l.id == 0 && l.budget_id == 0));
        let open = revision_lines(vec![line(day(2027, 1, 1), 5)], day(2027, 4, 10), true);
        assert!(!open[0].locked);
    }

    #[test]
    fn the_month_of_a_revision_day_is_not_locked() {
        let copied = revision_lines(vec![line(day(2027, 4, 1), 5)], day(2027, 4, 30), false);
        assert!(!copied[0].locked);
        let ended = revision_lines(vec![line(day(2027, 3, 1), 5)], day(2027, 4, 1), false);
        assert!(ended[0].locked);
    }

    #[test]
    fn csv_lines_split_on_the_delimiter_and_honour_quotes() {
        assert_eq!(
            split_csv("marketing, 2027-03 ,\"10,000.50\",\"say \"\"hi\"\"\"", ','),
            ["marketing", "2027-03", "10,000.50", "say \"hi\""]
        );
        assert_eq!(split_csv("a;b;", ';'), ["a", "b", ""]);
        assert_eq!(delimiter_of("category;month;amount\nx,y"), ';');
        assert_eq!(delimiter_of("category\tmonth\tamount"), '\t');
        assert_eq!(delimiter_of("category,month,amount"), ',');
        assert_eq!(delimiter_of(""), ',');
    }

    #[test]
    fn months_are_read_with_or_without_a_day() {
        assert_eq!(parse_month("2027-03"), Some(day(2027, 3, 1)));
        assert_eq!(parse_month(" 2027-03-17 "), Some(day(2027, 3, 1)));
        assert_eq!(parse_month("2027-13"), None);
        assert_eq!(parse_month("March"), None);
        assert_eq!(parse_month(""), None);
    }

    #[test]
    fn the_import_messages_name_the_value_and_the_rows() {
        let text = fill_text("wl_invoicing.budget.import.unknown_category", "fuell");
        assert!(text.contains("fuell"), "{text}");
        let pair = fill_pair(3, 5);
        assert!(pair.contains('3') && pair.contains('5'), "{pair}");
    }
}
