//! The payer contract (05 `CLN.INS.01`, minimal): the terms a payer pays a
//! plan under and the tariff of its services. Charge capture reads it to price
//! the insured part of a charge and to split it between the patient and the
//! payer ([`crate::split`]).
//!
//! Decision: the spec lists plans, networks, tariffs, copay and deductible
//! rules and coverage exclusions. The base keeps what the split needs: one
//! contract per payer and optionally per plan, with a validity period, a copay
//! and a deductible, the id of the payer agreement in wl-invoicing the claims
//! are billed under (`PayerBills::create`), and a tariff row per service: the
//! payer's price (when none, the self-pay price list is used) and whether the
//! service is covered at all. Networks and per-service copays come later.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::split::Terms;

/// A payer's contract for a plan.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_payer_contracts", index(payer_party_id))]
pub struct PayerContract {
    /// The primary key.
    pub id: i64,
    /// The payer, a party.
    pub payer_party_id: i64,
    /// The plan it applies to; `None` for every plan of the payer.
    pub plan: Option<String>,
    /// The payer agreement (its id in `wl-invoicing`) the claims are billed
    /// under; `None` until the billing coordinator sets it.
    pub agreement_id: Option<i64>,
    /// The patient's share of the covered amount after the deductible, in
    /// basis points (2000 = 20%).
    pub copay_bp: i64,
    /// The deductible of a visit, in fils.
    pub deductible: i64,
    /// The first day the contract applies.
    pub valid_from: NaiveDate,
    /// The last day it applies; `None` while open ended.
    pub valid_to: Option<NaiveDate>,
    /// The user who recorded it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerContract {
    /// The payer as a [`PartyId`].
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }

    /// Whether the contract applies on `day` (its last day is included).
    pub fn applies_on(&self, day: NaiveDate) -> bool {
        self.valid_from <= day && self.valid_to.is_none_or(|to| to >= day)
    }

    /// The terms the split reads.
    pub fn terms(&self) -> Terms {
        Terms {
            copay_bp: self.copay_bp,
            deductible: self.deductible,
        }
    }
}

/// What a contract says of one service.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_payer_tariffs",
    unique(contract_id, service_code)
)]
pub struct PayerTariff {
    /// The primary key.
    pub id: i64,
    /// The contract.
    #[model(references = "clinics_payer_contracts")]
    pub contract_id: i64,
    /// The service code.
    pub service_code: String,
    /// The payer's price of one unit in fils; `None` uses the self-pay price
    /// list.
    pub price: Option<i64>,
    /// Whether the payer covers the service; `false` is an exclusion, which
    /// the patient pays in full.
    pub covered: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PayerTariff {
    /// The tariff of service `code` under contract `contract`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of(db: &Db, contract: i64, code: &str) -> Result<Option<PayerTariff>> {
        PayerTariff::query()
            .where_(PayerTariff::CONTRACT_ID.eq(contract))
            .where_(PayerTariff::SERVICE_CODE.eq(code))
            .first(db)
            .await
    }

    /// The tariff rows of contract `contract`, by service code.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_contract(db: &Db, contract: i64) -> Result<Vec<PayerTariff>> {
        PayerTariff::query()
            .where_(PayerTariff::CONTRACT_ID.eq(contract))
            .order_by(PayerTariff::SERVICE_CODE)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    #[test]
    fn a_contract_applies_through_its_last_day() {
        let contract = PayerContract {
            valid_from: day("2026-01-01"),
            valid_to: Some(day("2026-12-31")),
            ..PayerContract::default()
        };
        assert!(contract.applies_on(day("2026-01-01")));
        assert!(contract.applies_on(day("2026-12-31")));
        assert!(!contract.applies_on(day("2027-01-01")));
        assert!(!contract.applies_on(day("2025-12-31")));
    }

    #[test]
    fn the_terms_are_the_contracts() {
        let contract = PayerContract {
            copay_bp: 2_000,
            deductible: 5_000,
            ..PayerContract::default()
        };
        assert_eq!(
            contract.terms(),
            Terms {
                copay_bp: 2_000,
                deductible: 5_000
            }
        );
    }
}
