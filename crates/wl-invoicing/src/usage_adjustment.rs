//! Hours corrected after billing (wl-invoicing §5.2 step 5, `FIN.ACL.01`).
//!
//! An approved adjustment timesheet arrives as `TimesheetApproved` of kind
//! `adjustment`. When its `affects` is `billing` or `both` and the timesheet
//! it `adjusts` was billed from usage records ([`crate::usage`]), each hours
//! category of the event (already netted by wl-attendance: the + and −
//! entries of one category are summed) is applied to the record of that
//! category, at that record's own price, multipliers and rate date, never
//! today's rate:
//!
//! - the record is not yet invoiced: its hours change, so the draft bills
//!   them with the original (a draft invoice that already holds the line is
//!   corrected, with its totals);
//! - invoiced, fewer hours: a credit note against the original invoice
//!   (`credit_partly`, which leaves the invoice itself as it is) for the
//!   hours at the line's price, whose reason names the adjustment. A credit
//!   above what the invoice can still be credited by is refused and the
//!   coordinator gets a task ([`TaskKind::AdjustmentCreditRefused`]);
//! - invoiced, more hours: an extra usage record that names the adjustment,
//!   billed on the next usage invoice, or at once on a draft invoice of its
//!   own when the setting [`AdjustmentPlus::SeparateInvoice`] says so.
//!
//! One row per adjustment and record ([`UsageAdjustment`]) makes an event
//! received twice change nothing. An adjustment that affects payroll only is
//! ignored. A dispute resolved `credited` with the adjustment links to the
//! credit note ([`credit_note_of`], used by [`crate::dispute_resolve`]).

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::attendance::TimesheetApproved;
use wl_kernel::invoicing::{CreditLine, InvoiceStatus};
use wl_kernel::notify::fill;
use wl_kernel::Subject;

use crate::allocate::amount_text;
use crate::config::AdjustmentPlus;
use crate::config_of;
use crate::credit_notes::{By, credit_partly};
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::task::{InvoicingTask, TaskKind, TaskStatus};
use crate::texts::english;
use crate::usage::{UsageRecord, bill_records};
use crate::vat::VatTotals;

/// What an adjustment did to one usage record.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AdjustmentOutcome {
    /// Claimed; the work is not finished (or a crash left it so).
    #[default]
    Pending,
    /// The record was not invoiced: its hours changed.
    Quantity,
    /// The record is on a draft invoice: the draft line was corrected.
    DraftLine,
    /// Fewer hours on an invoiced record: a credit note was issued.
    CreditNote,
    /// More hours on an invoiced record: an extra usage record waits for the
    /// next usage invoice.
    ExtraLine,
    /// More hours on an invoiced record: a separate draft invoice was made.
    ExtraInvoice,
    /// The adjustment could not be applied (a task was opened when an
    /// invoice was involved).
    Refused,
}

/// What an approved adjustment timesheet did to one usage record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_usage_adjustments",
    unique(adjustment_type, adjustment_id, usage_record_id),
    index(invoice_id)
)]
pub struct UsageAdjustment {
    /// The primary key.
    pub id: i64,
    /// The type of the adjustment timesheet.
    pub adjustment_type: String,
    /// The id of the adjustment timesheet.
    pub adjustment_id: i64,
    /// The type of the timesheet it corrects.
    pub adjusts_type: String,
    /// The id of the timesheet it corrects.
    pub adjusts_id: i64,
    /// The usage record it was applied to.
    pub usage_record_id: i64,
    /// `client_dispute`, `attendance_error`...
    pub origin: Option<String>,
    /// The signed change, in thousandths of an hour.
    pub delta_milli: i64,
    /// What it did.
    pub outcome: AdjustmentOutcome,
    /// The invoice the original record was billed on.
    pub invoice_id: Option<i64>,
    /// The credit note issued for fewer hours.
    pub credit_note_id: Option<i64>,
    /// The extra record made for more hours.
    pub extra_record_id: Option<i64>,
    /// The separate draft invoice made for more hours.
    pub extra_invoice_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl UsageAdjustment {
    /// The adjustment timesheet, `attendance.timesheet:7`.
    pub fn adjustment(&self) -> Subject {
        Subject::new(&self.adjustment_type, self.adjustment_id)
    }

    /// The timesheet it corrects.
    pub fn adjusts(&self) -> Subject {
        Subject::new(&self.adjusts_type, self.adjusts_id)
    }
}

/// Thousandths of an hour in `minutes`, rounded half away from zero.
pub fn milli_of(minutes: i64) -> i64 {
    let scaled = i128::from(minutes) * 1000;
    let rounded = if scaled < 0 {
        -((-scaled * 2 + 60) / 120)
    } else {
        (scaled * 2 + 60) / 120
    };
    i64::try_from(rounded).unwrap_or(0)
}

/// `milli` thousandths of an hour as text: `8`, `6.5`, `0.25`.
pub fn hours_text(milli: i64) -> String {
    let sign = if milli < 0 { "-" } else { "" };
    let whole = milli.abs() / 1000;
    let part = milli.abs() % 1000;
    if part == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = format!("{part:03}");
        format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
    }
}

/// The signed minutes of each hours category of `event`, summed.
fn netted(event: &TimesheetApproved) -> BTreeMap<String, i64> {
    let mut by_category: BTreeMap<String, i64> = BTreeMap::new();
    for total in &event.totals {
        *by_category.entry(total.category.clone()).or_insert(0) += total.minutes;
    }
    by_category
}

/// The text of a refusal.
fn text_of(error: Error) -> String {
    match error {
        Error::Status(_, message) => message,
        other => other.to_string(),
    }
}

/// The adjustments applied for `adjustment`, oldest first.
pub async fn of_adjustment(db: &Db, adjustment: &Subject) -> Result<Vec<UsageAdjustment>> {
    UsageAdjustment::query()
        .where_(UsageAdjustment::ADJUSTMENT_TYPE.eq(adjustment.subject_type.as_str()))
        .where_(UsageAdjustment::ADJUSTMENT_ID.eq(adjustment.subject_id))
        .order_by(UsageAdjustment::ID)
        .get(db)
        .await
}

/// The credit note `adjustment` gave against invoice `invoice_id`, if any:
/// what a dispute resolved `credited` with that adjustment links to.
pub async fn credit_note_of(
    db: &Db,
    adjustment: &Subject,
    invoice_id: i64,
) -> Result<Option<i64>> {
    Ok(of_adjustment(db, adjustment)
        .await?
        .into_iter()
        .filter(|row| row.invoice_id == Some(invoice_id))
        .find_map(|row| row.credit_note_id))
}

/// Applies the approved adjustment timesheet `event` to the usage it
/// corrects. Returns the rows it wrote, one per record changed; an event
/// that is not an adjustment, affects payroll only, adjusts a timesheet
/// nothing was billed from, or was applied before, gives none.
///
/// # Errors
///
/// A database error. A credit the invoice cannot take is not one: it opens
/// the coordinator's task.
pub async fn apply(state: &AppState, event: &TimesheetApproved) -> Result<Vec<UsageAdjustment>> {
    if event.kind != "adjustment" {
        return Ok(Vec::new());
    }
    if !matches!(event.affects.as_deref(), Some("billing" | "both")) {
        return Ok(Vec::new());
    }
    let Some(adjusts) = event.adjusts.clone() else {
        return Ok(Vec::new());
    };
    let mut done = Vec::new();
    for (category, minutes) in netted(event) {
        let delta = milli_of(minutes);
        if delta == 0 {
            continue;
        }
        let original = UsageRecord::query()
            .where_(UsageRecord::SOURCE_TYPE.eq(adjusts.subject_type.as_str()))
            .where_(UsageRecord::SOURCE_ID.eq(adjusts.subject_id))
            .where_(UsageRecord::CATEGORY.eq(category.as_str()))
            .where_(UsageRecord::ADJUSTS_ID.is_null())
            .order_by(UsageRecord::ID)
            .first(&state.db)
            .await?;
        let Some(record) = original else {
            continue;
        };
        let mut row = UsageAdjustment {
            adjustment_type: event.timesheet.subject_type.clone(),
            adjustment_id: event.timesheet.subject_id,
            adjusts_type: adjusts.subject_type.clone(),
            adjusts_id: adjusts.subject_id,
            usage_record_id: record.id,
            origin: event.origin.clone(),
            delta_milli: delta,
            outcome: AdjustmentOutcome::Pending,
            invoice_id: record.invoice_id,
            ..Default::default()
        };
        match row.insert(&state.db).await {
            Ok(()) => {}
            // Received twice: one credit note or line per adjustment.
            Err(error) if error.is_unique_violation() => continue,
            Err(error) => return Err(error),
        }
        settle(state, &mut row, record).await?;
        row.save(&state.db).await?;
        done.push(row);
    }
    Ok(done)
}

/// Applies `row` to `record` and stores what it did on `row`.
async fn settle(state: &AppState, row: &mut UsageAdjustment, record: UsageRecord) -> Result {
    let invoice = match record.invoice_id {
        Some(id) => Invoice::find(&state.db, id).await?,
        None => None,
    };
    match invoice {
        None => change_hours(state, row, record, None).await,
        Some(invoice) if invoice.status == InvoiceStatus::Draft => {
            change_hours(state, row, record, Some(invoice)).await
        }
        Some(invoice) if row.delta_milli < 0 => credit(state, row, &record, &invoice).await,
        Some(_) => extra(state, row, &record).await,
    }
}

/// Not yet invoiced (or only on a draft): the record's hours change, and the
/// draft line with them.
async fn change_hours(
    state: &AppState,
    row: &mut UsageAdjustment,
    mut record: UsageRecord,
    draft: Option<Invoice>,
) -> Result {
    let hours = record.quantity_milli + row.delta_milli;
    if hours < 0 {
        row.outcome = AdjustmentOutcome::Refused;
        return Ok(());
    }
    record.quantity_milli = hours;
    let mut tx = state.db.begin().await?;
    record.save(&mut tx).await?;
    if let (Some(mut invoice), Some(position)) = (draft, record.invoice_position) {
        let line = InvoiceLine::query()
            .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
            .where_(InvoiceLine::POSITION.eq(position))
            .first(&mut tx)
            .await?;
        if let Some(mut line) = line {
            line.quantity_milli = hours;
            line.set_amounts(&line.compute(invoice.aed_rate()));
            line.save(&mut tx).await?;
            let lines = InvoiceLine::query()
                .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
                .get(&mut tx)
                .await?;
            let amounts: Vec<_> = lines.iter().map(InvoiceLine::amounts).collect();
            invoice.set_totals(&VatTotals::of(&amounts));
            invoice.save(&mut tx).await?;
        }
        row.outcome = AdjustmentOutcome::DraftLine;
    } else {
        row.outcome = AdjustmentOutcome::Quantity;
    }
    tx.commit().await?;
    Ok(())
}

/// Fewer hours on an invoiced record: a credit note against the original
/// invoice for the hours at the line's own price.
async fn credit(
    state: &AppState,
    row: &mut UsageAdjustment,
    record: &UsageRecord,
    invoice: &Invoice,
) -> Result {
    let Some(position) = record.invoice_position else {
        row.outcome = AdjustmentOutcome::Refused;
        return Ok(());
    };
    let net = record.draft_line_for(-row.delta_milli).net();
    let reason = fill(
        english("wl_invoicing.usage_adjustment.credit_reason"),
        &json!({
            "adjustment": row.adjustment().to_string(),
            "origin": row.origin.clone().unwrap_or_default(),
            "adjusts": row.adjusts().to_string(),
            "hours": hours_text(-row.delta_milli),
        }),
    );
    let result = credit_partly(
        state,
        invoice.id,
        &[CreditLine::new(position, net)],
        &reason,
        By::System(None),
    )
    .await;
    match result {
        Ok(note) => {
            row.outcome = AdjustmentOutcome::CreditNote;
            row.credit_note_id = Some(note.id);
        }
        Err(error) => {
            row.outcome = AdjustmentOutcome::Refused;
            let why = text_of(error.into_error());
            open_task(state, row, invoice, net, &why).await?;
        }
    }
    Ok(())
}

/// More hours on an invoiced record: an extra record that names the
/// adjustment, billed on the next usage invoice or on a draft of its own.
async fn extra(state: &AppState, row: &mut UsageAdjustment, record: &UsageRecord) -> Result {
    let description = fill(
        english("wl_invoicing.usage_adjustment.extra_line"),
        &json!({
            "description": record.description,
            "hours": hours_text(row.delta_milli),
            "adjustment": row.adjustment().to_string(),
            "adjusts": row.adjusts().to_string(),
        }),
    );
    let adjustment = row.adjustment();
    let mut extra = UsageRecord {
        description,
        quantity_milli: row.delta_milli,
        adjusts_id: Some(record.id),
        adjustment_type: Some(adjustment.subject_type),
        adjustment_id: Some(adjustment.subject_id),
        invoice_id: None,
        invoice_position: None,
        ..record.clone()
    };
    extra.id = 0;
    extra.created_at = None;
    extra.updated_at = None;
    extra.insert(&state.db).await?;
    row.extra_record_id = Some(extra.id);
    if config_of(state).adjustment_plus == AdjustmentPlus::SeparateInvoice {
        let invoice = bill_records(&state.db, std::slice::from_mut(&mut extra)).await?;
        row.extra_invoice_id = Some(invoice.id);
        row.outcome = AdjustmentOutcome::ExtraInvoice;
    } else {
        row.outcome = AdjustmentOutcome::ExtraLine;
    }
    Ok(())
}

/// Opens the coordinator's task for a credit that was refused (once per
/// adjustment and record).
async fn open_task(
    state: &AppState,
    row: &UsageAdjustment,
    invoice: &Invoice,
    net: i64,
    reason: &str,
) -> Result {
    let adjustment = row.adjustment();
    let message = fill(
        english(TaskKind::AdjustmentCreditRefused.text_key()),
        &json!({
            "adjustment": adjustment.to_string(),
            "amount": amount_text(&invoice.currency, net),
            "number": invoice.number.clone().unwrap_or_default(),
            "reason": reason,
        }),
    );
    let mut task = InvoicingTask {
        kind: TaskKind::AdjustmentCreditRefused,
        invoice_id: Some(invoice.id),
        source_type: adjustment.subject_type.clone(),
        source_id: adjustment.subject_id,
        amount: net,
        key: format!("{adjustment}:{}", row.usage_record_id),
        message,
        role: config_of(state).adjustment_task_role.clone(),
        status: TaskStatus::Open,
        ..Default::default()
    };
    match task.insert(&state.db).await {
        Ok(()) => Ok(()),
        Err(error) if error.is_unique_violation() => Ok(()),
        Err(error) => Err(error),
    }
}

/// Registers the listener of `TimesheetApproved`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: TimesheetApproved, state: AppState| async move {
        apply(&state, &event).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::attendance::CategoryMinutes;
    use wl_kernel::workforce::EmployeeId;

    fn event(totals: &[(&str, i64)]) -> TimesheetApproved {
        TimesheetApproved {
            timesheet: Subject::new("attendance.timesheet", 9),
            owner: EmployeeId(1),
            period_from: renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            period_to: renox::chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap(),
            context: None,
            totals: totals
                .iter()
                .map(|(category, minutes)| CategoryMinutes {
                    category: (*category).to_owned(),
                    minutes: *minutes,
                })
                .collect(),
            approver_kind: "internal".into(),
            approval_basis: "approved".into(),
            kind: "adjustment".into(),
            adjusts: Some(Subject::new("attendance.timesheet", 2)),
            origin: Some("client_dispute".into()),
            affects: Some("billing".into()),
        }
    }

    #[test]
    fn minutes_become_thousandths_of_an_hour() {
        assert_eq!(milli_of(480), 8_000);
        assert_eq!(milli_of(-480), -8_000);
        assert_eq!(milli_of(90), 1_500);
        assert_eq!(milli_of(1), 17);
        assert_eq!(milli_of(-1), -17);
    }

    #[test]
    fn hours_read_without_trailing_zeros() {
        assert_eq!(hours_text(8_000), "8");
        assert_eq!(hours_text(6_500), "6.5");
        assert_eq!(hours_text(250), "0.25");
        assert_eq!(hours_text(-8_000), "-8");
    }

    #[test]
    fn entries_of_one_category_are_netted() {
        let sums = netted(&event(&[("normal", -600), ("normal", 120), ("overtime", 60)]));
        assert_eq!(sums.get("normal"), Some(&-480));
        assert_eq!(sums.get("overtime"), Some(&60));
    }
}
