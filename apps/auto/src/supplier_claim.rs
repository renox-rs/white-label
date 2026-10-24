//! The supplier warranty claim: the recovery of a failed part from the
//! supplier (08 §4.6 step 4, §6 *Supplier warranty claim*, §7.1 *Supplier
//! warranty claim*, `AUT.WAR.02`).
//!
//! It is raised from a warranty ticket (category `warranty`, wl-ticketing)
//! that was closed as accepted, for a part that carries a supplier warranty
//! ([`crate::supplier_terms`]). The supplier answers it (accepted or rejected,
//! recorded on the claim); it ends recovered (the supplier credited it, or a
//! replacement came) or written off. Its status moves only through the machine
//! in [`crate::supplier_claim_machine`]; [`crate::supplier_claims`] holds the
//! services.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

pub use crate::seed::SUPPLIER_CLAIM_SUBJECT_TYPE;

/// Where a supplier claim stands (08 §7.1 *Supplier warranty claim*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SupplierClaimStatus {
    /// The claim document was sent to the supplier.
    #[default]
    Raised,
    /// The supplier credited or replaced the part (terminal).
    Recovered,
    /// The supplier refused; the cost is absorbed as warranty expense
    /// (terminal).
    WrittenOff,
}

/// What the supplier said to the claim while it is raised.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SupplierAnswer {
    /// No answer yet.
    #[default]
    Pending,
    /// The supplier accepted it; the credit or the replacement follows.
    Accepted,
    /// The supplier refused it; the claim is written off or argued.
    Rejected,
}

/// A supplier warranty claim.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "auto_supplier_claims",
    state_machine,
    unique(ticket_id),
    index(supplier_party_id),
    index(supplier_return_id),
    index(status)
)]
pub struct SupplierClaim {
    /// The primary key.
    pub id: i64,
    /// The warranty ticket (wl-ticketing) the claim comes from.
    pub ticket_id: i64,
    /// Its number, `TKT-2026-0001`.
    pub ticket_number: String,
    /// The type of the sale or job card the ticket is about.
    pub about_type: Option<String>,
    /// The id of the sale or job card the ticket is about.
    pub about_id: Option<i64>,
    /// The supplier (a party).
    pub supplier_party_id: i64,
    /// The failed part.
    pub part_description: String,
    /// Its item code.
    pub item_code: Option<String>,
    /// Its brand.
    pub brand: Option<String>,
    /// The quantity in thousandths.
    pub quantity_milli: i64,
    /// What the part cost, in fils.
    pub part_cost: i64,
    /// Whether the terms include the labour of the replacement.
    pub labour_included: bool,
    /// The labour of the replacement that is claimed, in fils.
    pub labour_amount: i64,
    /// The amount claimed: the part, and the labour when it is included.
    pub claim_amount: i64,
    /// The day the part was bought from the supplier.
    pub purchase_date: NaiveDate,
    /// The warranty period in months.
    pub warranty_months: i64,
    /// The warranty mileage in kilometres, when the terms set one.
    pub warranty_km: Option<i64>,
    /// The job card the part was fitted on.
    pub fitted_job_card_id: Option<i64>,
    /// The job card of the replacement.
    pub replacement_job_card_id: Option<i64>,
    /// The vehicle's mileage when the part was fitted.
    pub fitted_odometer: Option<i64>,
    /// The day the part failed.
    pub failure_date: NaiveDate,
    /// The vehicle's mileage at the failure.
    pub failure_odometer: Option<i64>,
    /// Whether the failed part was kept: it goes back on a Return to supplier.
    pub part_kept: bool,
    /// The supplier's reference for the claim.
    pub supplier_reference: Option<String>,
    /// The claim document sent to the supplier.
    pub claim_document: Option<String>,
    /// The Return to supplier (wl-inventory) of the kept part.
    pub supplier_return_id: Option<i64>,
    /// When the claim document was queued to the supplier.
    pub claim_sent_at: Option<DateTime>,
    /// When the purchaser and the branch manager were told that the claim is
    /// not recovered at 60 days (`AUT.RUL.08`, once).
    pub overdue_notified_at: Option<DateTime>,
    /// What the supplier said.
    pub supplier_answer: SupplierAnswer,
    /// A note with the answer.
    pub supplier_answer_note: Option<String>,
    /// When the supplier answered.
    pub answered_at: Option<DateTime>,
    /// When the claim was raised.
    pub raised_at: Option<DateTime>,
    /// When it was recovered.
    pub recovered_at: Option<DateTime>,
    /// How: `credit_note`, `replacement`, `refund_received` or `manual`.
    pub recovery_basis: Option<String>,
    /// The supplier's credit note or the receipt of the replacement.
    pub recovery_reference: Option<String>,
    /// When it was written off.
    pub written_off_at: Option<DateTime>,
    /// Why it was written off.
    pub write_off_reason: Option<String>,
    /// The branch manager who approved the write-off.
    pub write_off_approved_by: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: SupplierClaimStatus,
    /// The user who raised it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SupplierClaim {
    /// The number people read: `SWC-0007`.
    pub fn number(&self) -> String {
        format!("SWC-{:04}", self.id)
    }

    /// The claim as a [`Subject`]: `auto.supplier_claim:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUPPLIER_CLAIM_SUBJECT_TYPE, self.id)
    }

    /// The sale or job card the ticket is about, when it is known.
    pub fn about(&self) -> Option<Subject> {
        match (&self.about_type, self.about_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// Whether the claim is raised and waits for the supplier.
    pub fn is_open(&self) -> bool {
        self.status == SupplierClaimStatus::Raised
    }

    /// The claim of warranty ticket `ticket_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_ticket(db: &Db, ticket_id: i64) -> Result<Option<SupplierClaim>> {
        SupplierClaim::query()
            .where_(SupplierClaim::TICKET_ID.eq(ticket_id))
            .first(db)
            .await
    }

    /// The open claim whose Return to supplier is `supplier_return_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_return(db: &Db, supplier_return_id: i64) -> Result<Option<SupplierClaim>> {
        SupplierClaim::query()
            .where_(SupplierClaim::SUPPLIER_RETURN_ID.eq(Some(supplier_return_id)))
            .where_(SupplierClaim::STATUS.eq(SupplierClaimStatus::Raised))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_is_numbered_and_is_a_subject() {
        let claim = SupplierClaim {
            id: 7,
            about_type: Some("auto.job_card".to_owned()),
            about_id: Some(3),
            ..SupplierClaim::default()
        };
        assert_eq!(claim.number(), "SWC-0007");
        assert_eq!(claim.subject().to_string(), "auto.supplier_claim:7");
        assert_eq!(
            claim.about().map(|s| s.to_string()),
            Some("auto.job_card:3".to_owned())
        );
        assert_eq!(SupplierClaim::default().about(), None);
    }

    #[test]
    fn a_new_claim_is_raised_and_waits_for_the_supplier() {
        let claim = SupplierClaim::default();
        assert!(claim.is_open());
        assert_eq!(claim.supplier_answer, SupplierAnswer::Pending);
    }
}
