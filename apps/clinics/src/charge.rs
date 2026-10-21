//! The charge line of a visit (05 §6 *Visit*, `CLN.ENC.01`): a service the
//! practitioner delivered or a consumable issued to the patient.
//!
//! A line is entered while the visit is open ([`crate::charges`]) and priced
//! when the visit is billed ([`crate::billing`]): the price of one unit, the
//! units that go self-pay because no prior authorisation covers them, and the
//! patient's and the payer's parts. A line is never deleted: a line taken out
//! (the procedure that was not performed) keeps its `removed_on` and is
//! ignored by the bill.
//!
//! Decision: the line carries its VAT treatment as a flag, `vat_standard`
//! (5%); the default is zero-rated, as treatment that is medically necessary
//! is (05 §13 question 5). The service catalogue (`SAL.CAT.01`) does not
//! provide the category yet; the practitioner or coder sets it when the
//! service is standard rated (cosmetic).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a charge line.
pub const CHARGE_SUBJECT_TYPE: &str = "clinics.charge_line";

/// What a line charges.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChargeKind {
    /// A service the practitioner recorded as delivered.
    #[default]
    Service,
    /// A consumable issued to the visit.
    Consumable,
}

/// A line of a visit's charge capture.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_charge_lines", index(visit_id))]
pub struct ChargeLine {
    /// The primary key.
    pub id: i64,
    /// The visit.
    #[model(references = "clinics_visits")]
    pub visit_id: i64,
    /// A service or a consumable.
    pub kind: ChargeKind,
    /// The service or consumable code.
    pub code: String,
    /// The units.
    pub units: i64,
    /// The price of one unit charged to the payer-covered units, in fils:
    /// the payer's tariff or the self-pay price. 0 for a clinic stop.
    pub unit_price: i64,
    /// The price of one unit before a clinic stop made it 0.
    pub list_price: i64,
    /// The units no prior authorisation covers: the patient pays them in full.
    pub self_pay_units: i64,
    /// The self-pay price of one unit of those, in fils.
    pub self_pay_price: i64,
    /// The units a prior authorisation covers (consumed when the bill is
    /// made).
    pub authorised_units: i64,
    /// Standard rated (5%); otherwise zero-rated.
    pub vat_standard: bool,
    /// The patient's part of the line, in fils.
    pub patient_share: i64,
    /// The payer's part of the line, in fils.
    pub payer_share: i64,
    /// Why the line is not charged (`clinic_stop`, `package`).
    pub reason: Option<String>,
    /// The prepaid package that paid the line.
    pub package_id: Option<i64>,
    /// The day the line was taken out (Asia/Dubai).
    pub removed_on: Option<NaiveDate>,
    /// Why (`not_suitable`).
    pub removed_reason: Option<String>,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ChargeLine {
    /// The line as a [`Subject`]: `clinics.charge_line:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(CHARGE_SUBJECT_TYPE, self.id)
    }

    /// Whether the line counts: it was not taken out.
    pub fn is_active(&self) -> bool {
        self.removed_on.is_none()
    }

    /// The units the payer's cover applies to: all but the self-pay units.
    pub fn covered_units(&self) -> i64 {
        (self.units - self.self_pay_units).max(0)
    }

    /// The amount the cover applies to, in fils.
    pub fn covered_total(&self) -> i64 {
        self.covered_units() * self.unit_price
    }

    /// The amount the patient pays in full, in fils.
    pub fn self_pay_total(&self) -> i64 {
        self.self_pay_units * self.self_pay_price
    }

    /// What the line costs: the covered and the self-pay parts, in fils.
    pub fn total(&self) -> i64 {
        self.covered_total() + self.self_pay_total()
    }

    /// The active lines of visit `visit`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn active_of(db: &Db, visit: i64) -> Result<Vec<ChargeLine>> {
        Ok(ChargeLine::of_visit(db, visit)
            .await?
            .into_iter()
            .filter(ChargeLine::is_active)
            .collect())
    }

    /// Every line of visit `visit`, oldest first, the removed ones too.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_visit(db: &Db, visit: i64) -> Result<Vec<ChargeLine>> {
        ChargeLine::query()
            .where_(ChargeLine::VISIT_ID.eq(visit))
            .order_by(ChargeLine::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_costs_its_covered_and_its_self_pay_units() {
        let line = ChargeLine {
            units: 3,
            unit_price: 10_000,
            self_pay_units: 1,
            self_pay_price: 12_000,
            ..ChargeLine::default()
        };
        assert_eq!(line.covered_units(), 2);
        assert_eq!(line.covered_total(), 20_000);
        assert_eq!(line.self_pay_total(), 12_000);
        assert_eq!(line.total(), 32_000);
        assert!(line.is_active());
    }
}
