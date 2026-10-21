//! Crediting a dispute (wl-invoicing §5.10, #67 A11, A12): the credit note
//! a `credited` resolution issues, and `FIN.ACL.07`.
//!
//! [`split`] spreads the agreed gross amount over the disputed lines. The
//! draft credit note is then created and checked against the
//! `invoicing.dispute_credit` policy by [`crate::dispute_resolve`]; the part
//! after the credit note is issued is [`finish_in`], which both paths share:
//!
//! - the direct path (no request needed) issues the note, then calls
//!   [`finish_in`];
//! - at the final approval the kernel runs the credit note's system
//!   `issue_approved`, whose effect is [`approved_in`]: it re-checks the
//!   stored invoice, lines and dispute, writes what `issue` writes, then
//!   calls [`finish_in`].
//!
//! [`finish_in`] uses the credit as #71 does: a `full` note cancels the
//! invoice ([`crate::credit::cancel_in`]), a partial one is allocated to it
//! and applied when used up. Then the invoice's flag is cleared and the
//! dispute stores its outcome, all in the caller's transaction.

use std::collections::HashMap;

use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, InvoiceStatus, VatCategory};

use crate::credit::cancel_in;
use crate::credit_note::{CreditNote, CreditNoteKind};
use crate::credit_note_line::CreditNoteLine;
use crate::credit_note_machine::write_issue_in;
use crate::credit_notes::{By, Outcomes, allocate_step, plan_lines};
use crate::dispute::Dispute;
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::vat::{div_round_half_up, line_vat};

/// The approval policies' action key of a dispute credit (`FIN.ACL.07`).
pub const ACTION: &str = "invoicing.dispute_credit";

/// The invoice states a credit note can be issued against.
const CREDITABLE: [InvoiceStatus; 4] = [
    InvoiceStatus::Issued,
    InvoiceStatus::PartiallyPaid,
    InvoiceStatus::Overdue,
    InvoiceStatus::Paid,
];

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The request's payload: the amount the policy reads and the dispute the
/// credit settles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Payload {
    /// The credit note's gross total, in fils.
    pub amount: i64,
    /// The dispute's id.
    pub dispute: i64,
}

/// A disputed invoice line, as [`split`] sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitLine {
    /// The line's position on the invoice.
    pub position: i64,
    /// Its VAT treatment.
    pub category: VatCategory,
    /// The net that can still be credited on it (#71 A4).
    pub remaining_net: i64,
}

/// The gross of a credit of `net` fils in `category`.
fn gross_of(net: i64, category: VatCategory) -> i64 {
    net + line_vat(net, category)
}

/// The net of a credit line whose gross should be `gross`: the gross divided
/// by one plus the rate, rounded half-up, then moved by one fil when that
/// matches `gross` better. The VAT is rounded per line, so some grosses
/// cannot be hit exactly; the nearest one not above `gross` is taken then.
fn net_for(gross: i64, category: VatCategory, most: i64) -> i64 {
    let rate = i128::from(category.rate_bp());
    let guess = div_round_half_up(i128::from(gross) * 10_000, 10_000 + rate).clamp(0, most);
    [guess, guess - 1, guess + 1]
        .into_iter()
        .filter(|net| (0..=most).contains(net))
        .min_by_key(|net| {
            let off = gross_of(*net, category) - gross;
            // The nearest gross; one above is worse than one below.
            (off.abs(), off > 0)
        })
        .unwrap_or(guess)
}

/// Spreads the `agreed` gross amount over `lines`, in position order. Each
/// line takes up to its remaining creditable gross; the last line used takes
/// what is left, as a net of `gross × 10000 ÷ (10000 + rate)` rounded
/// half-up and then adjusted by a fil when that makes net plus VAT match.
/// Returns the credit lines (net per invoice line position).
///
/// # Errors
///
/// The amount is not above 0, or it is above what the lines can still be
/// credited by (together with VAT).
pub fn split(lines: &[SplitLine], agreed: i64) -> std::result::Result<Vec<CreditLine>, String> {
    if agreed <= 0 {
        return Err("The agreed amount must be above 0.".to_owned());
    }
    let mut sorted = lines.to_vec();
    sorted.sort_by_key(|line| line.position);
    let capacity: i64 = sorted
        .iter()
        .map(|line| gross_of(line.remaining_net.max(0), line.category))
        .sum();
    if agreed > capacity {
        return Err(format!(
            "The agreed amount is above what can still be credited on the disputed lines ({capacity} fils with VAT)."
        ));
    }
    let mut left = agreed;
    let mut credits = Vec::new();
    for line in &sorted {
        if left == 0 {
            break;
        }
        let most = line.remaining_net.max(0);
        let room = gross_of(most, line.category);
        if room == 0 {
            continue;
        }
        let net = if left >= room {
            most
        } else {
            net_for(left, line.category, most)
        };
        if net == 0 {
            continue;
        }
        left -= gross_of(net, line.category);
        credits.push(CreditLine::new(line.position, net));
    }
    if credits.is_empty() {
        return Err("The agreed amount is too small to credit.".to_owned());
    }
    Ok(credits)
}

/// Why `origins` (the origin type of each disputed line) cannot be credited
/// yet: a line billed from a timesheet needs an approved adjustment
/// timesheet (`attendance.timesheet`, origin `client_dispute`), whose credit note
/// is linked by [`crate::dispute_resolve`] (#86). Otherwise it is refused, with or
/// without a timesheet named.
pub fn timesheet_guard<'a>(
    origins: impl IntoIterator<Item = Option<&'a str>>,
) -> std::result::Result<(), String> {
    if origins
        .into_iter()
        .any(|origin| origin == Some("attendance.timesheet"))
    {
        Err("A line billed from a timesheet needs an approved adjustment timesheet.".to_owned())
    } else {
        Ok(())
    }
}

/// The net that issued credit notes credited, by invoice line id, read in
/// `tx`, without credit note `except`: the one being issued has already
/// moved to `issued` when its effect runs.
async fn credited_in(
    tx: &mut renox::db::Transaction,
    invoice_id: i64,
    except: i64,
) -> Result<HashMap<i64, i64>> {
    let issued: Vec<i64> = CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice_id))
        .where_(CreditNote::STATUS.is_in([CreditNoteStatus::Issued, CreditNoteStatus::Applied]))
        .get(&mut *tx)
        .await?
        .into_iter()
        .map(|n| n.id)
        .filter(|id| *id != except)
        .collect();
    let mut by_line = HashMap::new();
    if issued.is_empty() {
        return Ok(by_line);
    }
    let lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.is_in(issued))
        .get(&mut *tx)
        .await?;
    for line in lines {
        *by_line.entry(line.invoice_line_id).or_insert(0) += line.net_amount;
    }
    Ok(by_line)
}

/// The checks `issue_approved` repeats against the stored rows: the dispute
/// is still open and holds this credit note, the invoice can be credited and
/// is issued by an active entity, and every line is within its remaining
/// credit.
async fn check_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    payload: &Payload,
) -> Result<Dispute> {
    let dispute = Dispute::find(&mut *tx, payload.dispute)
        .await?
        .ok_or_else(|| refused("The dispute no longer exists."))?;
    if !dispute.is_open() || dispute.credit_note_id != Some(note.id) {
        return Err(refused("The dispute was already resolved."));
    }
    let invoice = Invoice::find_or_404(&mut *tx, note.invoice_id).await?;
    if !invoice.disputed || invoice.dispute_id != Some(dispute.id) {
        return Err(refused("The invoice is no longer disputed."));
    }
    if !CREDITABLE.contains(&invoice.status) {
        return Err(refused(format!(
            "An invoice that is {} cannot be credited.",
            invoice.status
        )));
    }
    let entity = match invoice.legal_entity_id {
        Some(id) => LegalEntity::find(&mut *tx, id).await?,
        None => None,
    };
    if note.legal_entity_id != invoice.legal_entity_id
        || !entity.as_ref().is_some_and(LegalEntity::is_active)
    {
        return Err(refused("The issuing legal entity is not active."));
    }
    let invoice_lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .order_by(InvoiceLine::POSITION)
        .get(&mut *tx)
        .await?;
    let stored = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(note.id))
        .order_by(CreditNoteLine::POSITION)
        .get(&mut *tx)
        .await?;
    let requested: Vec<CreditLine> = stored
        .iter()
        .filter_map(|line| {
            invoice_lines
                .iter()
                .find(|l| l.id == line.invoice_line_id)
                .map(|l| CreditLine::new(l.position, line.net_amount))
        })
        .collect();
    if requested.is_empty() || requested.len() != stored.len() {
        return Err(refused(
            "A line of the credit note is no longer on the invoice.",
        ));
    }
    let credited = credited_in(tx, invoice.id, note.id).await?;
    plan_lines(&requested, &invoice_lines, &credited).map_err(refused)?;
    Ok(dispute)
}

/// What follows the issue of credit note `issued`, in `tx`: the credit is
/// used on the invoice (a full note cancels it, a partial one is allocated
/// and applied when used up), then the invoice's flag is cleared and the
/// dispute stores its outcome. `resolver` is the user who resolved the
/// dispute. Returns the note as it is then.
///
/// # Errors
///
/// The refusals of the credit services, and a 422 when the dispute was
/// already resolved. The caller rolls `tx` back.
pub(crate) async fn finish_in(
    tx: &mut renox::db::Transaction,
    issued: CreditNote,
    dispute_id: i64,
    resolver: Option<i64>,
    by: &By<'_>,
    outcomes: &mut Outcomes,
) -> std::result::Result<CreditNote, TransitionError> {
    let invoice = Invoice::find_or_404(&mut *tx, issued.invoice_id).await?;
    let done = match issued.kind {
        CreditNoteKind::Full => {
            let reason = issued.reason.clone();
            cancel_in(tx, &issued, &invoice, &reason, false, by, outcomes).await?
        }
        CreditNoteKind::Partial => allocate_step(tx, issued, by, outcomes).await?,
    };
    settle_in(tx, invoice.id, dispute_id, done.id, resolver).await?;
    Ok(done)
}

/// Clears the invoice's flag and stores the `credited` outcome on the
/// dispute, only while both are still open.
pub(crate) async fn settle_in(
    tx: &mut renox::db::Transaction,
    invoice_id: i64,
    dispute_id: i64,
    credit_note_id: i64,
    resolver: Option<i64>,
) -> Result {
    let now = renox::db::now();
    let cleared = renox::db::sql(format!(
        "UPDATE {} SET disputed = 0, dispute_id = NULL, updated_at = ? \
         WHERE id = ? AND disputed = 1 AND dispute_id = ?",
        Invoice::TABLE
    ))
    .bind(now)
    .bind(invoice_id)
    .bind(dispute_id)
    .execute(&mut *tx)
    .await?;
    if cleared == 0 {
        return Err(refused("The dispute was already resolved."));
    }
    let settled = renox::db::sql(format!(
        "UPDATE {} SET outcome = ?, credit_note_id = ?, resolved_by_user_id = ?, \
         resolved_at = ?, updated_at = ? WHERE id = ? AND outcome IS NULL",
        Dispute::TABLE
    ))
    .bind(wl_kernel::invoicing::DisputeOutcome::Credited.as_str())
    .bind(credit_note_id)
    .bind(resolver)
    .bind(now)
    .bind(now)
    .bind(dispute_id)
    .execute(&mut *tx)
    .await?;
    if settled == 0 {
        return Err(refused("The dispute was already resolved."));
    }
    Ok(())
}

/// The effect of `issue_approved`: checks the stored rows again, writes what
/// `issue` writes, and finishes the credit as the system on behalf of the
/// approver. The events of the nested runs are emitted after the commit by
/// [`crate::events`].
pub(crate) async fn approved_in(
    tx: &mut renox::db::Transaction,
    note: &CreditNote,
    input: &Value,
    meta: &Value,
) -> Result {
    let payload: Payload = renox::serde_json::from_value(input.clone())
        .map_err(|_| refused("The approval names no dispute."))?;
    let dispute = check_in(tx, note, &payload).await?;
    write_issue_in(tx, note).await?;
    let issued = CreditNote::find_or_404(&mut *tx, note.id).await?;
    let approver = meta.get("actor_id").and_then(Value::as_i64);
    let requester = meta.get("requester_id").and_then(Value::as_i64);
    let mut outcomes = Outcomes::default();
    finish_in(
        tx,
        issued,
        dispute.id,
        requester,
        &By::System(approver),
        &mut outcomes,
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(position: i64, category: VatCategory, remaining_net: i64) -> SplitLine {
        SplitLine {
            position,
            category,
            remaining_net,
        }
    }

    /// The gross the credit lines add up to.
    fn gross(lines: &[SplitLine], credits: &[CreditLine]) -> i64 {
        credits
            .iter()
            .map(|c| {
                let l = lines.iter().find(|l| l.position == c.invoice_line).unwrap();
                gross_of(c.net, l.category)
            })
            .sum()
    }

    #[test]
    fn a_whole_line_is_credited_at_its_net() {
        let lines = [line(1, VatCategory::Standard, 100_000)];
        let credits = split(&lines, 105_000).unwrap();
        assert_eq!(credits, [CreditLine::new(1, 100_000)]);
    }

    #[test]
    fn the_amount_fills_the_lines_in_position_order() {
        // Gross 840.00 on line 1 and 420.00 on line 2.
        let lines = [
            line(2, VatCategory::Standard, 40_000),
            line(1, VatCategory::Standard, 80_000),
        ];
        let credits = split(&lines, 100_000).unwrap();
        assert_eq!(
            credits,
            [CreditLine::new(1, 80_000), CreditLine::new(2, 15_238)]
        );
        assert_eq!(gross(&lines, &credits), 100_000);
        // Less than the first line: only the first is used.
        let one = split(&lines, 52_500).unwrap();
        assert_eq!(one, [CreditLine::new(1, 50_000)]);
    }

    #[test]
    fn the_net_is_the_gross_over_one_plus_the_rate_rounded_half_up() {
        let lines = [line(1, VatCategory::Standard, 1_000_000)];
        // 1,200.00 ÷ 1.05 = 1,142.857…
        assert_eq!(
            split(&lines, 120_000).unwrap(),
            [CreditLine::new(1, 114_286)]
        );
        // 0.01 ÷ 1.05 = 0.0095: one fil of net, no VAT.
        assert_eq!(split(&lines, 1).unwrap(), [CreditLine::new(1, 1)]);
        let zero = [line(1, VatCategory::ZeroRated, 1_000_000)];
        assert_eq!(split(&zero, 777).unwrap(), [CreditLine::new(1, 777)]);
    }

    #[test]
    fn the_gross_matches_when_rounding_allows_and_is_never_above() {
        let lines = [line(1, VatCategory::Standard, 10_000_000)];
        for agreed in 1..=4_000 {
            let credits = split(&lines, agreed).unwrap();
            let total = gross(&lines, &credits);
            assert!(
                total <= agreed && agreed - total <= 1,
                "{agreed} gave {total}"
            );
            let reachable = (0..=agreed).any(|net| gross_of(net, VatCategory::Standard) == agreed);
            if reachable {
                assert_eq!(total, agreed, "{agreed} is reachable");
            }
        }
    }

    #[test]
    fn more_than_the_remaining_credit_is_refused() {
        // 300.00 of 1,000.00 was credited before: 700.00 + 35.00 VAT is left.
        let lines = [
            line(1, VatCategory::Standard, 70_000),
            line(2, VatCategory::Standard, 0),
        ];
        assert!(split(&lines, 73_500).is_ok());
        let error = split(&lines, 73_501).unwrap_err();
        assert!(error.contains("73500 fils"), "{error}");
        assert!(split(&[], 100).is_err());
        assert!(split(&lines, 0).is_err());
        assert!(split(&lines, -5).is_err());
    }

    #[test]
    fn a_timesheet_line_needs_an_adjustment_timesheet() {
        assert!(timesheet_guard([Some("property.charge"), None]).is_ok());
        let error = timesheet_guard([None, Some("attendance.timesheet")]).unwrap_err();
        assert!(error.contains("approved adjustment timesheet"));
    }

    #[test]
    fn the_payload_carries_the_amount_and_the_dispute() {
        let payload = Payload {
            amount: 120_000,
            dispute: 4,
        };
        let json = renox::serde_json::to_value(&payload).unwrap();
        assert_eq!(json["amount"], 120_000);
        assert_eq!(
            renox::serde_json::from_value::<Payload>(json).unwrap(),
            payload
        );
    }
}
