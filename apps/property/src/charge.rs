//! The payment plan and the charge schedule of a lease version
//! (`PRP.LSE.01`, `FIN.BIL.01`).
//!
//! Until wl-invoicing keeps charge schedules itself (the base of BL-PRP-06),
//! the lease keeps its own: one [`LeaseCharge`] per instalment, planned while
//! the version is drafted, *scheduled* when it is activated, *invoiced* by the
//! daily run on its due date through the kernel `Invoicer` (one tax invoice
//! per instalment, whose due date is also its tax point, §13 question 4), and
//! *cancelled* when the version ends or is replaced.
//!
//! A cheque is linked to the charge it pays with the charge's [`Subject`]
//! (`property.lease_charge:<id>`), so the cheque register can say what each
//! post-dated cheque is for ([`crate::cheques`]).

use renox::chrono::{Months, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{Subject, VatCategory};

/// The subject type of a charge of a lease.
pub const CHARGE_SUBJECT_TYPE: &str = "property.lease_charge";

/// The charge type of the rent instalments.
pub const RENT: &str = "rent";

/// Where a charge stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChargeStatus {
    /// In the payment plan of a version that is not active yet.
    #[default]
    Planned,
    /// Billable: the invoice is made on its due date.
    Scheduled,
    /// A tax invoice was made for it.
    Invoiced,
    /// Not billed, because the version ended or was replaced.
    Cancelled,
}

/// One instalment or other scheduled amount of a lease version.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_lease_charges", index(lease_id, position), index(status, due_date))]
pub struct LeaseCharge {
    /// The primary key.
    pub id: i64,
    /// The lease version.
    #[model(references = "property_leases")]
    pub lease_id: i64,
    /// 1, 2, ...: the order in the plan.
    pub position: i32,
    /// `rent`, or a code of the client's charge catalogue.
    pub charge_type: String,
    /// The line text on the invoice.
    pub description: String,
    /// The due date, which is also the tax point.
    pub due_date: NaiveDate,
    /// The amount before VAT, in fils.
    pub amount: i64,
    /// The VAT treatment of the line.
    pub vat_category: VatCategory,
    /// Where it stands.
    pub status: ChargeStatus,
    /// The invoice made for it, once invoiced.
    pub invoice_id: Option<i64>,
    /// Created by `expire` for a period the tenant holds over (`PRP.RUL.03`).
    pub holding_over: bool,
    /// The legal entity that issues the invoice, when it is not the default:
    /// the new owner's entity of a charge recreated when its unit moved to
    /// another agreement (BL-PRP-23).
    pub legal_entity_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LeaseCharge {
    /// The charge as a kernel subject: what a cheque is linked to.
    pub fn subject(&self) -> Subject {
        Subject::new(CHARGE_SUBJECT_TYPE, self.id)
    }

    /// The charges of a version, in order.
    pub async fn of_lease(db: &Db, lease_id: i64) -> Result<Vec<LeaseCharge>> {
        LeaseCharge::query()
            .where_(LeaseCharge::LEASE_ID.eq(lease_id))
            .order_by(LeaseCharge::POSITION)
            .order_by(LeaseCharge::ID)
            .get(db)
            .await
    }
}

/// The rent plan of `instalments` payments that add up to `annual_rent`:
/// the first falls on `first_due`, the others every `12 / instalments` months
/// after it, counted from the first so that a month end never drifts (31 Jan,
/// 28 Feb, 31 Mar). The amount is split in whole fils; the last instalment
/// takes the remainder.
///
/// Nothing for 0 instalments or an annual rent of 0 or less.
pub fn rent_plan(
    first_due: NaiveDate,
    annual_rent: i64,
    instalments: u32,
) -> Vec<(NaiveDate, i64)> {
    if instalments == 0 || annual_rent <= 0 {
        return Vec::new();
    }
    let n = i64::from(instalments);
    let each = annual_rent / n;
    (0..instalments)
        .map(|i| {
            let months = 12 * i / instalments;
            let due = first_due
                .checked_add_months(Months::new(months))
                .unwrap_or(first_due);
            let amount = if i + 1 == instalments {
                annual_rent - each * (n - 1)
            } else {
                each
            };
            (due, amount)
        })
        .collect()
}

/// The text of a rent line: `Rent 3/12`.
pub fn rent_description(number: i32, of: usize) -> String {
    format!("Rent {number}/{of}")
}

/// Whether `charges` (the live ones) add up to `annual_rent` in rent.
pub fn plan_total(charges: &[LeaseCharge]) -> i64 {
    charges
        .iter()
        .filter(|c| c.charge_type == RENT && c.status != ChargeStatus::Cancelled)
        .map(|c| c.amount)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn four_cheques_a_year_fall_every_three_months() {
        let plan = rent_plan(day(2027, 1, 1), 10_000_000, 4);
        let dates: Vec<_> = plan.iter().map(|(d, _)| *d).collect();
        assert_eq!(
            dates,
            [day(2027, 1, 1), day(2027, 4, 1), day(2027, 7, 1), day(2027, 10, 1)]
        );
        assert!(plan.iter().all(|(_, amount)| *amount == 2_500_000));
    }

    #[test]
    fn the_last_instalment_takes_the_remainder() {
        let plan = rent_plan(day(2027, 1, 1), 1_000_001, 3);
        let amounts: Vec<_> = plan.iter().map(|(_, a)| *a).collect();
        assert_eq!(amounts, [333_333, 333_333, 333_335]);
        assert_eq!(amounts.iter().sum::<i64>(), 1_000_001);
    }

    #[test]
    fn a_month_end_does_not_drift() {
        let plan = rent_plan(day(2027, 1, 31), 1_200_000, 12);
        let dates: Vec<_> = plan.iter().map(|(d, _)| *d).collect();
        assert_eq!(dates[0], day(2027, 1, 31));
        assert_eq!(dates[1], day(2027, 2, 28));
        assert_eq!(dates[2], day(2027, 3, 31));
        assert_eq!(dates[11], day(2027, 12, 31));
    }

    #[test]
    fn five_instalments_are_spread_over_the_year() {
        let plan = rent_plan(day(2027, 1, 1), 500_000, 5);
        let months: Vec<_> = plan.iter().map(|(d, _)| d.format("%m").to_string()).collect();
        assert_eq!(months, ["01", "03", "05", "08", "10"]);
    }

    #[test]
    fn nothing_without_instalments_or_rent() {
        assert!(rent_plan(day(2027, 1, 1), 100, 0).is_empty());
        assert!(rent_plan(day(2027, 1, 1), 0, 4).is_empty());
    }

    #[test]
    fn the_plan_total_skips_cancelled_and_other_charges() {
        let charge = |amount, status, kind: &str| LeaseCharge {
            amount,
            status,
            charge_type: kind.to_owned(),
            ..Default::default()
        };
        let charges = [
            charge(100, ChargeStatus::Planned, RENT),
            charge(50, ChargeStatus::Cancelled, RENT),
            charge(30, ChargeStatus::Planned, "parking"),
            charge(70, ChargeStatus::Invoiced, RENT),
        ];
        assert_eq!(plan_total(&charges), 170);
    }
}
