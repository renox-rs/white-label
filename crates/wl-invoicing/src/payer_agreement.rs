//! The Payer agreement (wl-invoicing §3): the terms with a third-party payer
//! (an insurer, a TPA) that a [payer bill](crate::payer_bill::PayerBill)
//! follows: who gets the tax invoice, whether a payer reference is required,
//! the first-submission and resubmission windows, the resubmission limit,
//! the write-off limit (`FIN.ACL.14`) and whether the customer may be billed
//! for what the payer will not pay.
//!
//! Decision: the agreement here carries only what the payer bill lifecycle
//! reads. Its rate card, invoice wording and remittance import preset come
//! with `payer_agreements.manage` (a later issue); a vertical or the host
//! stores the row until then.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

/// Who the tax invoice of a payer bill is addressed to.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VatRecipient {
    /// The customer: one tax invoice for the full supply with two
    /// receivable shares; the payer bill is a claim statement (05 default).
    #[default]
    Customer,
    /// The payer: a tax invoice to the payer for its share, and one to the
    /// customer for the rest (08 default).
    Payer,
}

/// The terms with one payer. Money is `i64` fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_payer_agreements", index(payer_party_id))]
pub struct PayerAgreement {
    /// The primary key.
    pub id: i64,
    /// The legal entity the agreement is with.
    pub legal_entity_id: i64,
    /// The payer (a party).
    pub payer_party_id: i64,
    /// The agreement's name, e.g. `Daman Gold`.
    pub name: String,
    /// Whether new payer bills may be issued under it.
    pub active: bool,
    /// Who gets the tax invoice.
    pub vat_recipient: VatRecipient,
    /// Whether a payer bill needs its payer reference (an approval number)
    /// to be issued.
    pub reference_required: bool,
    /// Days after the service in which the payer accepts a first claim.
    pub first_submission_days: i64,
    /// Days after a remittance in which the payer accepts a resubmission.
    pub resubmission_days: i64,
    /// How many resubmissions the payer accepts.
    pub resubmission_limit: i64,
    /// A write-off above this many fils needs an approval (`FIN.ACL.14`).
    pub write_off_limit: i64,
    /// Whether the payer still accepts a first claim after the window.
    pub accepts_late_submissions: bool,
    /// Whether what the payer will not pay may be billed to the customer.
    pub bill_customer_allowed: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerAgreement {
    /// The payer.
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }

    /// The last day a first submission is accepted for a service on
    /// `service_date` (Asia/Dubai, inclusive).
    pub fn first_submission_deadline(&self, service_date: NaiveDate) -> Option<NaiveDate> {
        add_days(service_date, self.first_submission_days)
    }

    /// The last day a resubmission is accepted after a remittance dated
    /// `remitted_on` (Asia/Dubai, inclusive).
    pub fn resubmission_deadline(&self, remitted_on: NaiveDate) -> Option<NaiveDate> {
        add_days(remitted_on, self.resubmission_days)
    }

    /// Whether the first-submission window of a service on `service_date`
    /// has passed on `today`: `today` is after its last day.
    pub fn first_window_passed(&self, service_date: NaiveDate, today: NaiveDate) -> bool {
        self.first_submission_deadline(service_date)
            .is_some_and(|last| today > last)
    }
}

/// `date` plus `days` days; `None` for a negative count or an overflow.
fn add_days(date: NaiveDate, days: i64) -> Option<NaiveDate> {
    date.checked_add_days(Days::new(u64::try_from(days).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn agreement() -> PayerAgreement {
        PayerAgreement {
            first_submission_days: 30,
            resubmission_days: 15,
            ..Default::default()
        }
    }

    #[test]
    fn the_first_window_ends_on_its_last_day_inclusive() {
        let agreement = agreement();
        let visit = day("2026-09-01");
        assert_eq!(
            agreement.first_submission_deadline(visit),
            Some(day("2026-10-01"))
        );
        assert!(!agreement.first_window_passed(visit, day("2026-10-01")));
        assert!(agreement.first_window_passed(visit, day("2026-10-02")));
    }

    #[test]
    fn the_resubmission_window_counts_from_the_remittance() {
        assert_eq!(
            agreement().resubmission_deadline(day("2026-12-10")),
            Some(day("2026-12-25"))
        );
    }

    #[test]
    fn recipients_are_stored_in_snake_case() {
        let names: Vec<_> = VatRecipient::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(names, ["customer", "payer"]);
    }
}
