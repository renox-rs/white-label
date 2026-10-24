//! The insurer: the terms the workshop has with an insurance company (08 §6
//! *Insurer*, `AUT.INS.01`, `AUT.INS.02`).
//!
//! The insurer itself is a party (wl-parties); this row holds what the
//! workshop agreed with it: the payment terms, the claim email, the
//! surveyors' contacts, the labour rate and the storage rate, and the payer
//! agreement of wl-invoicing its claims are billed under (`FIN.TPP.01`). The
//! insurer's rate card (`FIN.RAT.01`) is kept by wl-invoicing for the source
//! [`Insurer::subject`] (`auto.insurer:N`) and read through the kernel's
//! `RateCards` contract ([`crate::insurers::rate_card_of`]).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

pub use crate::seed::INSURER_SUBJECT_TYPE;

/// An insurer the workshop repairs for.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_insurers", unique(party_id), index(active))]
pub struct Insurer {
    /// The primary key.
    pub id: i64,
    /// The insurance company (a party).
    pub party_id: i64,
    /// The payer agreement in wl-invoicing the claims are billed under.
    pub payer_agreement_id: Option<i64>,
    /// Where estimates, supplementaries and withdrawals are emailed.
    pub claim_email: Option<String>,
    /// The surveyors and how to reach them.
    pub surveyor_contacts: Option<String>,
    /// Days the insurer takes to pay.
    pub payment_terms_days: i64,
    /// The hourly labour rate agreed, in fils.
    pub labour_rate: Option<i64>,
    /// The daily storage rate agreed, in fils (0 or none: no storage charge).
    pub storage_rate_daily: Option<i64>,
    /// Whether new claims may be registered for it.
    pub active: bool,
    /// The user who recorded it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Insurer {
    /// The insurer as a [`Subject`]: `auto.insurer:7`, the source of its rate
    /// card.
    pub fn subject(&self) -> Subject {
        Subject::new(INSURER_SUBJECT_TYPE, self.id)
    }

    /// The insurer of party `party_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_party(db: &Db, party_id: i64) -> Result<Option<Insurer>> {
        Insurer::query()
            .where_(Insurer::PARTY_ID.eq(party_id))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_insurer_is_the_source_of_its_rate_card() {
        let insurer = Insurer {
            id: 4,
            ..Insurer::default()
        };
        assert_eq!(insurer.subject().to_string(), "auto.insurer:4");
    }
}
