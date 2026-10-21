//! The BOQ lines a variation can omit work from, and the progress an IPA
//! records on them (02 Construction §4.5, §6, §7.1 *Variation* `submit`;
//! BL-CON-12). The BOQ and the calculation of an IPA come with their own
//! issues; this is the part an omission needs: a line with its quantity and
//! rate, and the **cumulative quantity done** on it as the latest recorded IPA
//! shows it.
//!
//! The **latest recorded progress** is the project's `draft` IPA if it has
//! one, else its latest `submitted`, `certified` or `invoiced` IPA by number
//! ([`progress_ipa`]). The **quantity not yet done** of a line is its quantity
//! less the progress on that IPA ([`not_yet_done`]). Quantities are whole
//! units of the line's unit, money is fils.

use renox::db::{DateTime, Db};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ipa::{Ipa, IpaStatus};

/// A line of the BOQ of a project.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_boq_lines", index(project_id))]
pub struct BoqLine {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// The line number in the BOQ, e.g. `4.2`.
    pub number: String,
    /// What the line is.
    pub description: String,
    /// The unit of the quantity, e.g. `m²`.
    pub unit: String,
    /// The quantity of the contract.
    pub quantity: i64,
    /// The rate per unit, in fils.
    pub rate: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BoqLine {
    /// A line `number` of `project_id` with `quantity` units at `rate` fils.
    pub fn new(project_id: i64, number: &str, quantity: i64, rate: i64) -> Self {
        BoqLine {
            project_id,
            number: number.trim().to_owned(),
            quantity,
            rate,
            ..BoqLine::default()
        }
    }
}

/// The cumulative quantity an IPA records on a BOQ line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_ipa_progress", index(ipa_id))]
pub struct IpaProgress {
    /// The primary key.
    pub id: i64,
    /// The IPA.
    pub ipa_id: i64,
    /// The line.
    pub boq_line_id: i64,
    /// The cumulative quantity done.
    pub quantity: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The IPA of `project_id` that shows the latest recorded progress: its
/// `draft` IPA, else its latest `submitted`, `certified` or `invoiced` one.
///
/// # Errors
///
/// A database error.
pub async fn progress_ipa(db: &Db, project_id: i64) -> Result<Option<Ipa>> {
    let draft = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.eq(IpaStatus::Draft))
        .order_by_desc(Ipa::NUMBER)
        .first(db)
        .await?;
    if draft.is_some() {
        return Ok(draft);
    }
    Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.is_in([
            IpaStatus::Submitted,
            IpaStatus::Certified,
            IpaStatus::Invoiced,
        ]))
        .order_by_desc(Ipa::NUMBER)
        .first(db)
        .await
}

/// The cumulative quantity done on `line` as the latest recorded IPA of its
/// project shows it; 0 without an IPA or without progress on the line.
///
/// # Errors
///
/// A database error.
pub async fn done_quantity(db: &Db, line: &BoqLine) -> Result<i64> {
    let Some(ipa) = progress_ipa(db, line.project_id).await? else {
        return Ok(0);
    };
    let progress = IpaProgress::query()
        .where_(IpaProgress::IPA_ID.eq(ipa.id))
        .where_(IpaProgress::BOQ_LINE_ID.eq(line.id))
        .first(db)
        .await?;
    Ok(progress.map_or(0, |found| found.quantity))
}

/// The quantity of `line` not yet done: its quantity less the progress of the
/// latest recorded IPA, never below 0.
///
/// # Errors
///
/// A database error.
pub async fn not_yet_done(db: &Db, line: &BoqLine) -> Result<i64> {
    Ok((line.quantity - done_quantity(db, line).await?).max(0))
}

/// Records `quantity` as the cumulative progress of IPA `ipa_id` on line
/// `boq_line_id`, replacing what the IPA recorded before.
///
/// # Errors
///
/// A database error.
pub async fn record_progress(
    db: &Db,
    ipa_id: i64,
    boq_line_id: i64,
    quantity: i64,
) -> Result<IpaProgress> {
    let existing = IpaProgress::query()
        .where_(IpaProgress::IPA_ID.eq(ipa_id))
        .where_(IpaProgress::BOQ_LINE_ID.eq(boq_line_id))
        .first(db)
        .await?;
    let mut row = existing.unwrap_or(IpaProgress {
        ipa_id,
        boq_line_id,
        ..IpaProgress::default()
    });
    row.quantity = quantity;
    row.save(db).await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_named_by_its_number() {
        let line = BoqLine::new(1, " 4.2 ", 1_000, 12_000);
        assert_eq!(line.number, "4.2");
        assert_eq!(line.quantity, 1_000);
        assert_eq!(line.rate, 12_000);
    }
}
