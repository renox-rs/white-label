//! The invoice of a delivered job card (08 §4.4 step 7, `FIN.INV.01`) and the
//! completion of its booking (`SCH.BKG.01`).
//!
//! On delivery the approved lines are billed to the customer through the
//! kernel's invoicer: each labour line at its price (an hourly one for its
//! hours at its rate, a flat-rate one at its flat price), each part with its
//! condition in the description (new, used or renovated, which the invoice
//! must state, 08 §2). The invoice is issued at once where invoicing is set to
//! issue the job card's invoices ([`crate::job_card::JOB_CARD_SUBJECT_TYPE`]),
//! otherwise it stays a draft for the accountant. The booking the job card was
//! opened from is completed as the system.
//!
//! Decisions: the delivery is committed first and the billing follows it; a
//! billing that fails leaves the job card `delivered` without an invoice,
//! shown on its page, and [`bill`] runs again from there (the claim on
//! `invoiced_at` keeps it to one invoice). A booking that cannot be completed
//! yet (its start has not passed) is left for the desk. A job card that was
//! billed has `invoice_id`; the lines billed are the approved ones.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::dubai_date;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, VatCategory, invoicer};
use wl_kernel::scheduling::{BookingId, booking_actions};

use crate::db::refused;
use crate::estimate::Estimate;
use crate::job_card::{JobCard, JobCardStatus};
use crate::labour_line::hours_text;
use crate::labour_line::{LabourLine, LabourPricing, LineStatus};
use crate::part_line::PartLine;
use crate::texts::english;

/// What follows the delivery of `card`: the booking is completed and the
/// invoice raised.
///
/// # Errors
///
/// The database's and the invoicer's errors.
pub async fn after_deliver(state: &AppState, card: &JobCard) -> Result {
    if let Some(booking) = card.booking_id {
        // The booking completes only after its start; the desk completes it
        // otherwise.
        let _ = booking_actions(state)
            .complete(state, BookingId(booking))
            .await;
    }
    bill(state, card.id).await?;
    Ok(())
}

/// The invoice lines of the approved labour: `Brake pads (1.5 h)`.
pub fn labour_draft_line(line: &LabourLine, origin: wl_kernel::Subject) -> DraftLine {
    let description = match line.pricing {
        LabourPricing::Hourly => english(
            "auto.billing.hourly",
            &[
                ("description", &line.description),
                ("hours", &hours_text(line.hours_milli)),
            ],
        ),
        LabourPricing::FlatRate => line.description.clone(),
    };
    let draft = DraftLine::new(&description, line.rate, VatCategory::Standard).origin(origin);
    match line.pricing {
        LabourPricing::Hourly => draft.quantity_milli(line.hours_milli),
        LabourPricing::FlatRate => draft,
    }
}

/// The invoice line of an approved part: `Brake pad set (new)`.
pub fn part_draft_line(line: &PartLine, origin: wl_kernel::Subject) -> DraftLine {
    let condition = english(
        &format!("auto.parts.condition.{}", line.part_condition.as_str()),
        &[],
    );
    let description = english(
        "auto.billing.part",
        &[
            ("description", &line.description),
            ("condition", &condition),
        ],
    );
    DraftLine::new(&description, line.unit_price, VatCategory::Standard)
        .quantity_milli(line.quantity_milli)
        .origin(origin)
}

/// The draft invoice of `card` for the lines the customer approved; `None`
/// when no line was approved.
pub fn draft_of(card: &JobCard, estimate: &Estimate) -> Option<InvoiceDraft> {
    let origin = card.subject();
    let mut draft = InvoiceDraft::new(PartyId(card.customer_party_id), origin.clone());
    if let Some(workshop) = card.workshop() {
        draft = draft.scope(workshop);
    }
    if let Some(at) = card.delivered_at {
        draft = draft.date_of_supply(dubai_date(at));
    }
    let mut lines = 0;
    for line in estimate
        .labour
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
    {
        draft = draft.line(labour_draft_line(line, origin.clone()));
        lines += 1;
    }
    for line in estimate
        .parts
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
    {
        draft = draft.line(part_draft_line(line, origin.clone()));
        lines += 1;
    }
    (lines > 0).then_some(draft)
}

/// Bills the delivered job card `job_card_id`: drafts the invoice for its
/// approved lines, issues it where the invoicing module is set to, and keeps
/// its id on the job card. A job card billed already answers its invoice
/// again; one with nothing approved, or with invoicing off, has none.
///
/// # Errors
///
/// A 422 for a job card that was not delivered; the database's and the
/// invoicer's errors (the claim on the job card is undone, so the billing can
/// run again).
pub async fn bill(state: &AppState, job_card_id: i64) -> Result<Option<InvoiceId>> {
    let card = JobCard::find_or_404(&state.db, job_card_id).await?;
    if let Some(id) = card.invoice_id {
        return Ok(Some(InvoiceId(id)));
    }
    if card.status != JobCardStatus::Delivered {
        return Err(refused(english("auto.billing.problem.not_delivered", &[])));
    }
    let estimate = Estimate::load(&state.db, card.id).await?;
    let Some(draft) = draft_of(&card, &estimate) else {
        return Ok(None);
    };
    // One run bills a job card: the claim is undone when the invoicer fails.
    let none: Option<renox::db::DateTime> = None;
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
    use crate::part_line::PartCondition;

    #[test]
    fn an_hourly_line_is_billed_for_its_hours_at_its_rate() {
        let line = LabourLine {
            description: "Diagnostics".to_owned(),
            pricing: LabourPricing::Hourly,
            hours_milli: 1_500,
            rate: 16_000,
            ..LabourLine::default()
        };
        let billed = labour_draft_line(&line, wl_kernel::Subject::new("auto.job_card", 3));
        assert_eq!(billed.quantity_milli, 1_500);
        assert_eq!(billed.unit_price, 16_000);
        assert_eq!(billed.net(), 24_000);
        assert_eq!(billed.description, "Diagnostics (1.5 h)");
    }

    #[test]
    fn a_flat_rate_line_is_billed_at_its_flat_price() {
        let line = LabourLine {
            description: "Oil change".to_owned(),
            pricing: LabourPricing::FlatRate,
            hours_milli: 1_000,
            rate: 12_000,
            ..LabourLine::default()
        };
        let billed = labour_draft_line(&line, wl_kernel::Subject::new("auto.job_card", 3));
        assert_eq!(billed.net(), 12_000);
        assert_eq!(billed.description, "Oil change");
    }

    #[test]
    fn a_part_is_billed_with_its_condition() {
        let line = PartLine {
            description: "Brake pad set".to_owned(),
            quantity_milli: 2_000,
            unit_price: 9_000,
            part_condition: PartCondition::Renovated,
            ..PartLine::default()
        };
        let billed = part_draft_line(&line, wl_kernel::Subject::new("auto.job_card", 3));
        assert_eq!(billed.description, "Brake pad set (renovated)");
        assert_eq!(billed.net(), 18_000);
    }

    #[test]
    fn only_approved_lines_are_billed() {
        let card = JobCard {
            id: 3,
            customer_party_id: 7,
            ..JobCard::default()
        };
        let approved = LabourLine {
            description: "Brakes".to_owned(),
            hours_milli: 1_000,
            rate: 20_000,
            approval: LineStatus::Approved,
            ..LabourLine::default()
        };
        let declined = LabourLine {
            approval: LineStatus::Declined,
            ..approved.clone()
        };
        let estimate = Estimate {
            labour: vec![approved, declined],
            parts: Vec::new(),
        };
        let draft = draft_of(&card, &estimate).expect("a draft");
        assert_eq!(draft.lines.len(), 1);
        assert_eq!(draft.party, PartyId(7));
        assert!(draft_of(&card, &Estimate::default()).is_none());
    }
}
