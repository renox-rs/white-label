//! The invoice of a job card that ended before delivery: the work done of a
//! cancelled job (`AUT.ACL.21`) or the inspection fee of a declined one
//! (`AUT.RUL.17`, BL-AUT-03). It is issued when the transition runs, not left
//! in draft (08 §7.1 *Job card*).
//!
//! The lines are those of [`crate::stop::draft_lines`] for the work the cancel
//! recorded; when the branch manager approved a bill below the work done, the
//! reduction is spread over the lines as discounts
//! ([`crate::stop::spread_reduction`]), so the invoice adds up to the amount
//! approved. A job card billed nothing (a waived bill, a decline with no fee)
//! has no invoice. One run bills a job card: the claim on `invoiced_at` is
//! undone when the invoicer fails, so the billing can run again.

use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, invoicer};

use crate::db::refused;
use crate::estimate::Estimate;
use crate::job_card::{JobCard, JobCardStatus};
use crate::stop;
use crate::texts::english;

/// The draft invoice of the closed job card `card` for what it was billed
/// (`billed_net`); `None` when nothing was.
pub fn draft_of(card: &JobCard, estimate: &Estimate) -> Option<InvoiceDraft> {
    if card.billed_net <= 0 {
        return None;
    }
    let mut lines = stop::draft_lines(card, estimate);
    let value: i64 = lines.iter().map(DraftLine::net).sum();
    if card.billed_net < value {
        stop::spread_reduction(&mut lines, value - card.billed_net);
    }
    if lines.is_empty() {
        return None;
    }
    let mut draft = InvoiceDraft::new(PartyId(card.customer_party_id), card.subject());
    if let Some(workshop) = card.workshop() {
        draft = draft.scope(workshop);
    }
    if let Some(at) = card.closed_at {
        draft = draft.date_of_supply(dubai_date(at));
    }
    for line in lines {
        draft = draft.line(line);
    }
    Some(draft)
}

/// Bills the declined or cancelled job card `job_card_id`: drafts the invoice,
/// issues it where the invoicing module is set to issue the job card's
/// invoices, and keeps its id on the job card. A job card billed already
/// answers its invoice again; one billed nothing, or with invoicing off, has
/// none.
///
/// # Errors
///
/// A 422 for a job card that is not declined or cancelled; the database's and
/// the invoicer's errors (the claim on the job card is undone, so the billing
/// can run again).
pub async fn bill_closed(state: &AppState, job_card_id: i64) -> Result<Option<InvoiceId>> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if let Some(id) = card.invoice_id {
        return Ok(Some(InvoiceId(id)));
    }
    if !matches!(
        card.status,
        JobCardStatus::Declined | JobCardStatus::Cancelled
    ) {
        return Err(refused(english("auto.stop.problem.not_closed", &[])));
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let Some(draft) = draft_of(&card, &estimate) else {
        return Ok(None);
    };
    let none: Option<DateTime> = None;
    let claimed = JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .where_(JobCard::INVOICED_AT.is_null())
        .update(&state.db, &[("invoiced_at", &Some(now()))])
        .await?;
    if claimed == 0 {
        let again = JobCard::find_or_404(&state.db, card.id).await?;
        return Ok(again.invoice_id.map(InvoiceId));
    }
    let books = invoicer(state);
    let drafted = match books.draft(&state.db, draft).await {
        Ok(drafted) => drafted,
        Err(error) => {
            JobCard::query()
                .where_(JobCard::ID.eq(card.id))
                .update(&state.db, &[("invoiced_at", &none)])
                .await?;
            return Err(error);
        }
    };
    let Some(id) = drafted else {
        // Invoicing is off: nothing was billed, and nothing is claimed.
        JobCard::query()
            .where_(JobCard::ID.eq(card.id))
            .update(&state.db, &[("invoiced_at", &none)])
            .await?;
        return Ok(None);
    };
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            &state.db,
            &[("invoice_id", &Some(id.0)), ("updated_at", &now())],
        )
        .await?;
    // The invoicing module issues a draft as the system only where it is set
    // to; otherwise the accountant issues it.
    let _issued = books.issue(state, id, None).await;
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labour_line::{LabourLine, LabourPricing, LineStatus, WorkStatus};

    fn card(billed: i64) -> JobCard {
        JobCard {
            id: 10,
            customer_party_id: 7,
            billed_net: billed,
            ..JobCard::default()
        }
    }

    fn estimate() -> Estimate {
        Estimate {
            labour: vec![LabourLine {
                id: 1,
                description: "Brake pads".to_owned(),
                pricing: LabourPricing::Hourly,
                hours_milli: 1_000,
                rate: 20_000,
                approval: LineStatus::Approved,
                work: WorkStatus::Done,
                hours_done_milli: 1_000,
                ..LabourLine::default()
            }],
            parts: Vec::new(),
        }
    }

    #[test]
    fn nothing_billed_nothing_drafted() {
        assert!(draft_of(&card(0), &estimate()).is_none());
    }

    #[test]
    fn the_draft_bills_the_work_done_or_the_amount_approved_below_it() {
        let full = draft_of(&card(20_000), &estimate()).expect("a draft");
        assert_eq!(full.party, PartyId(7));
        assert_eq!(full.lines.iter().map(DraftLine::net).sum::<i64>(), 20_000);
        let lowered = draft_of(&card(12_000), &estimate()).expect("a draft");
        assert_eq!(lowered.lines.iter().map(DraftLine::net).sum::<i64>(), 12_000);
    }
}
