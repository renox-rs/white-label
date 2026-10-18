//! Drafting credit notes (wl-invoicing §5.12): a partial credit note is
//! drafted against an invoice, changed and discarded while it is a `draft`.
//! A draft has no number and no effect on any balance; `issue` (its own
//! transition) makes it count.
//!
//! The credit on one invoice line is limited to its net minus what issued
//! credit notes already credited on it (the "remaining credit",
//! [`vat::remaining_credit`]). Drafts do not count against it, so several
//! drafts can be prepared and the first one issued wins.

use std::collections::HashMap;

use renox::prelude::*;
use wl_kernel::invoicing::{CreditLine, CreditNoteStatus, InvoiceStatus};

use crate::credit_note::{CreditNote, CreditNoteKind};
use crate::credit_note_line::CreditNoteLine;
use crate::invoice::Invoice;
use crate::line::InvoiceLine;
use crate::vat::{self, VatTotals};

/// The invoice states a credit note can be drafted against.
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

/// The invoice lines to credit, with the net on each, or why the credit
/// cannot be drafted: the invoice is not in a state that can be credited,
/// the reason is blank, there are no lines, a line is unknown, repeated or
/// not above 0, or its net is above the line's remaining credit.
/// `credited` is the net issued credit notes already credited, by invoice
/// line id.
pub fn plan<'a>(
    invoice_status: InvoiceStatus,
    reason: &str,
    requested: &[CreditLine],
    invoice_lines: &'a [InvoiceLine],
    credited: &HashMap<i64, i64>,
) -> std::result::Result<Vec<(&'a InvoiceLine, i64)>, String> {
    if !CREDITABLE.contains(&invoice_status) {
        return Err(format!(
            "an invoice that is {invoice_status} cannot be credited"
        ));
    }
    if reason.trim().is_empty() {
        return Err("give a reason for the credit note".to_owned());
    }
    if requested.is_empty() {
        return Err("a credit note needs at least one line".to_owned());
    }
    let mut planned: Vec<(&InvoiceLine, i64)> = Vec::with_capacity(requested.len());
    for credit in requested {
        let position = credit.invoice_line;
        let Some(line) = invoice_lines.iter().find(|l| l.position == position) else {
            return Err(format!("the invoice has no line {position}"));
        };
        if planned.iter().any(|(l, _)| l.id == line.id) {
            return Err(format!("line {position} is credited twice"));
        }
        if credit.net <= 0 {
            return Err(format!("line {position} must credit more than 0"));
        }
        let remaining = vat::remaining_credit(
            line.net_amount,
            credited.get(&line.id).copied().unwrap_or(0),
        );
        if credit.net > remaining {
            return Err(format!(
                "line {position} can be credited by {remaining} fils at most"
            ));
        }
        planned.push((line, credit.net));
    }
    Ok(planned)
}

/// The net that issued credit notes credited, by invoice line id.
pub async fn credited_by_line(db: &Db, invoice_id: i64) -> Result<HashMap<i64, i64>> {
    let issued: Vec<i64> = CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice_id))
        .where_(CreditNote::STATUS.is_in([CreditNoteStatus::Issued, CreditNoteStatus::Applied]))
        .get(db)
        .await?
        .into_iter()
        .map(|n| n.id)
        .collect();
    if issued.is_empty() {
        return Ok(HashMap::new());
    }
    let lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.is_in(issued))
        .get(db)
        .await?;
    let mut by_line = HashMap::new();
    for line in lines {
        *by_line.entry(line.invoice_line_id).or_insert(0) += line.net_amount;
    }
    Ok(by_line)
}

/// The lines of credit note `credit_note_id`, in order.
pub async fn lines_of(db: &Db, credit_note_id: i64) -> Result<Vec<CreditNoteLine>> {
    CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(credit_note_id))
        .order_by(CreditNoteLine::POSITION)
        .get(db)
        .await
}

/// The totals per VAT category of credit note `credit_note_id`, from its
/// stored lines.
pub async fn totals_of(db: &Db, credit_note_id: i64) -> Result<VatTotals> {
    let lines = lines_of(db, credit_note_id).await?;
    let amounts: Vec<_> = lines.iter().map(CreditNoteLine::amounts).collect();
    Ok(VatTotals::of(&amounts))
}

/// The lines for `planned` on credit note `credit_note_id`, and their totals.
fn build(
    credit_note_id: i64,
    planned: &[(&InvoiceLine, i64)],
    rate_micro: Option<i64>,
) -> (Vec<CreditNoteLine>, VatTotals) {
    let lines: Vec<CreditNoteLine> = planned
        .iter()
        .zip(1..)
        .map(|((line, net), position)| {
            CreditNoteLine::of(credit_note_id, position, line, *net, rate_micro)
        })
        .collect();
    let amounts: Vec<_> = lines.iter().map(CreditNoteLine::amounts).collect();
    let totals = VatTotals::of(&amounts);
    (lines, totals)
}

/// Plans `lines` on `invoice`, from its stored lines and the credit issued.
async fn plan_for(
    db: &Db,
    invoice: &Invoice,
    lines: &[CreditLine],
    reason: &str,
) -> Result<(Vec<InvoiceLine>, Vec<(i64, i64)>)> {
    let invoice_lines = crate::invoicer::lines_of(db, invoice.id).await?;
    let credited = credited_by_line(db, invoice.id).await?;
    let planned = plan(invoice.status, reason, lines, &invoice_lines, &credited)
        .map_err(refused)?
        .into_iter()
        .map(|(line, net)| (line.id, net))
        .collect();
    Ok((invoice_lines, planned))
}

/// The planned `(invoice line id, net)` pairs resolved against
/// `invoice_lines`.
fn resolve<'a>(
    invoice_lines: &'a [InvoiceLine],
    planned: &[(i64, i64)],
) -> Vec<(&'a InvoiceLine, i64)> {
    planned
        .iter()
        .filter_map(|(id, net)| {
            invoice_lines
                .iter()
                .find(|l| l.id == *id)
                .map(|l| (l, *net))
        })
        .collect()
}

/// Drafts a credit note against `invoice` for `lines` (the net on each
/// invoice line, by position) with its lines and totals in one
/// transaction. It is a `draft`: no number, no remaining balance, and the
/// invoice is not touched. The legal entity, currency and rate are the
/// invoice's.
///
/// Refused (422) for an invoice that is `draft`, `credited` or
/// `written_off`, a blank reason, no lines, an unknown or repeated line, a
/// net of 0 or less, or a net above the line's remaining credit.
pub async fn draft(
    db: &Db,
    invoice: &Invoice,
    lines: &[CreditLine],
    reason: &str,
    created_by: Option<i64>,
) -> Result<CreditNote> {
    // The invoice as it is now, not as the caller loaded it.
    let invoice = Invoice::find_or_404(db, invoice.id).await?;
    let (invoice_lines, planned) = plan_for(db, &invoice, lines, reason).await?;
    let mut note = CreditNote {
        invoice_id: invoice.id,
        legal_entity_id: invoice.legal_entity_id,
        party_id: invoice.party_id,
        currency: invoice.currency.clone(),
        exchange_rate_micro: invoice.exchange_rate_micro,
        reason: reason.trim().to_owned(),
        kind: CreditNoteKind::Partial,
        created_by,
        status: CreditNoteStatus::Draft,
        ..Default::default()
    };
    let (_, totals) = build(0, &resolve(&invoice_lines, &planned), note.aed_rate());
    note.set_totals(&totals);

    let mut tx = db.begin().await?;
    note.insert(&mut tx).await?;
    let (mut new_lines, _) = build(note.id, &resolve(&invoice_lines, &planned), note.aed_rate());
    for line in &mut new_lines {
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(note)
}

/// Changes the lines and the reason of draft `credit_note`, replacing its
/// lines and totals in one transaction. Same refusals as [`draft`], and 422
/// when it is no longer a `draft`.
pub async fn change(
    db: &Db,
    credit_note: &CreditNote,
    lines: &[CreditLine],
    reason: &str,
) -> Result<CreditNote> {
    let mut note = CreditNote::find_or_404(db, credit_note.id).await?;
    if note.status != CreditNoteStatus::Draft {
        return Err(refused("only a draft credit note can be changed"));
    }
    let invoice = Invoice::find_or_404(db, note.invoice_id).await?;
    let (invoice_lines, planned) = plan_for(db, &invoice, lines, reason).await?;
    let (mut new_lines, totals) =
        build(note.id, &resolve(&invoice_lines, &planned), note.aed_rate());
    note.reason = reason.trim().to_owned();
    note.set_totals(&totals);

    let mut tx = db.begin().await?;
    note.save(&mut tx).await?;
    let old_lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(note.id))
        .get(&mut tx)
        .await?;
    for mut old in old_lines {
        old.delete(&mut tx).await?;
    }
    for line in &mut new_lines {
        line.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(note)
}

/// Deletes draft `credit_note` and its lines. Refused (422) when it is no
/// longer a `draft`: an issued credit note is never deleted.
pub async fn discard(db: &Db, credit_note: &CreditNote) -> Result<()> {
    let mut note = CreditNote::find_or_404(db, credit_note.id).await?;
    if note.status != CreditNoteStatus::Draft {
        return Err(refused("only a draft credit note can be discarded"));
    }
    let mut tx = db.begin().await?;
    let old_lines = CreditNoteLine::query()
        .where_(CreditNoteLine::CREDIT_NOTE_ID.eq(note.id))
        .get(&mut tx)
        .await?;
    for mut old in old_lines {
        old.delete(&mut tx).await?;
    }
    note.delete(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::invoicing::VatCategory;

    fn lines() -> Vec<InvoiceLine> {
        vec![
            InvoiceLine {
                id: 11,
                position: 1,
                net_amount: 100_000,
                vat_category: VatCategory::Standard,
                ..Default::default()
            },
            InvoiceLine {
                id: 12,
                position: 2,
                net_amount: 50_000,
                vat_category: VatCategory::ZeroRated,
                ..Default::default()
            },
        ]
    }

    fn check(
        status: InvoiceStatus,
        reason: &str,
        requested: &[CreditLine],
        credited: &HashMap<i64, i64>,
    ) -> std::result::Result<Vec<(i64, i64)>, String> {
        let lines = lines();
        plan(status, reason, requested, &lines, credited)
            .map(|p| p.into_iter().map(|(l, n)| (l.id, n)).collect())
    }

    #[test]
    fn a_credit_within_the_line_is_planned() {
        let none = HashMap::new();
        let request = [CreditLine::new(1, 20_000), CreditLine::new(2, 50_000)];
        assert_eq!(
            check(InvoiceStatus::Issued, " Damaged ", &request, &none),
            Ok(vec![(11, 20_000), (12, 50_000)])
        );
        for status in CREDITABLE {
            assert!(check(status, "x", &request, &none).is_ok());
        }
    }

    #[test]
    fn the_credit_is_limited_by_what_was_credited() {
        let credited = HashMap::from([(11, 80_000)]);
        let at_most = check(
            InvoiceStatus::Issued,
            "x",
            &[CreditLine::new(1, 20_000)],
            &credited,
        );
        assert_eq!(at_most, Ok(vec![(11, 20_000)]));
        let over = check(
            InvoiceStatus::Issued,
            "x",
            &[CreditLine::new(1, 20_001)],
            &credited,
        );
        assert!(over.unwrap_err().contains("20000 fils at most"));
    }

    #[test]
    fn bad_credits_are_refused() {
        let none = HashMap::new();
        let ok = [CreditLine::new(1, 100)];
        for status in [
            InvoiceStatus::Draft,
            InvoiceStatus::Credited,
            InvoiceStatus::WrittenOff,
        ] {
            assert!(
                check(status, "x", &ok, &none)
                    .unwrap_err()
                    .contains("cannot be credited")
            );
        }
        assert!(
            check(InvoiceStatus::Issued, "  ", &ok, &none)
                .unwrap_err()
                .contains("reason")
        );
        assert!(
            check(InvoiceStatus::Issued, "x", &[], &none)
                .unwrap_err()
                .contains("at least one line")
        );
        let unknown = check(InvoiceStatus::Issued, "x", &[CreditLine::new(3, 1)], &none);
        assert!(unknown.unwrap_err().contains("no line 3"));
        let twice = [CreditLine::new(1, 1), CreditLine::new(1, 2)];
        assert!(
            check(InvoiceStatus::Issued, "x", &twice, &none)
                .unwrap_err()
                .contains("twice")
        );
        for net in [0, -5] {
            let zero = [CreditLine::new(2, net)];
            assert!(
                check(InvoiceStatus::Issued, "x", &zero, &none)
                    .unwrap_err()
                    .contains("more than 0")
            );
        }
    }

    #[test]
    fn lines_and_totals_are_built_per_line() {
        let invoice_lines = lines();
        let planned = [(&invoice_lines[0], 20_000), (&invoice_lines[1], 5_000)];
        let (built, totals) = build(9, &planned, Some(vat::RATE_SCALE));
        assert_eq!(built.len(), 2);
        assert_eq!((built[0].credit_note_id, built[0].position), (9, 1));
        assert_eq!((built[1].position, built[1].invoice_line_id), (2, 12));
        assert_eq!(built[0].vat_amount, 1_000);
        assert_eq!(built[1].vat_amount, 0);
        assert_eq!(
            (totals.net, totals.vat, totals.gross),
            (25_000, 1_000, 26_000)
        );
        assert_eq!(totals.vat_aed, Some(1_000));
    }
}
