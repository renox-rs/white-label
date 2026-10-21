//! The salary structure (wl-hr §3 *Salary structure*, §7 *Salary
//! components*): the pay agreed for an employee from a day on. The payroll
//! calculation (BL-HRM-16) reads it: the fixed wage of a line is the sum of
//! the components of the structure effective in the period, and hours are
//! paid at the structure's hourly rate.
//!
//! Decisions:
//!
//! - **Effective structure.** The structure with the latest `effective_from`
//!   on or before the day asked for. Hours added or taken back later (an
//!   adjustment timesheet, a reconciliation) are priced with the structure
//!   effective on the day of the hours, not today's (§5.3 step 2).
//! - **Hourly rate.** Set on the structure, else derived: the basic wage
//!   divided by 30 days of 8 hours ([`SalaryStructure::hourly`]). Overtime is
//!   paid on the basic wage in the UAE; allowances do not count.
//! - **Not built here.** Creating a structure with its approval threshold
//!   (`salary.change`, `SalaryChanged`) and the pro-rating of joiners and
//!   leavers belong to the salary and leave issues; a structure is stored
//!   with [`SalaryStructure::create`].

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// The days a month and the hours a day an hourly rate is derived from.
pub const DAYS_PER_MONTH: i64 = 30;
/// The hours of a working day.
pub const HOURS_PER_DAY: i64 = 8;

/// The pay agreed for an employee from `effective_from`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_salary_structures", unique(employee_id, effective_from))]
pub struct SalaryStructure {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The first day it applies.
    pub effective_from: NaiveDate,
    /// The basic wage for a month, AED fils.
    pub basic: i64,
    /// The housing allowance, AED fils.
    pub housing: i64,
    /// The transport allowance, AED fils.
    pub transport: i64,
    /// Other fixed allowances, AED fils.
    pub other_allowances: i64,
    /// The pay of one hour, AED fils; empty derives it from the basic wage.
    pub hourly_rate: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl SalaryStructure {
    /// The gross fixed wage for a month: basic plus the fixed allowances. The
    /// basis of the Art. 25 caps (`HRM.ACL.06`).
    pub fn fixed_wage(&self) -> i64 {
        self.basic + self.housing + self.transport + self.other_allowances
    }

    /// The pay of one hour, AED fils.
    pub fn hourly(&self) -> i64 {
        self.hourly_rate
            .unwrap_or(self.basic / (DAYS_PER_MONTH * HOURS_PER_DAY))
    }

    /// The structure of `employee` effective on `day`, inside `tx`.
    pub async fn effective_in(
        tx: &mut Transaction,
        employee: i64,
        day: NaiveDate,
    ) -> Result<Option<SalaryStructure>> {
        SalaryStructure::query()
            .where_(SalaryStructure::EMPLOYEE_ID.eq(employee))
            .where_(SalaryStructure::EFFECTIVE_FROM.lte(day))
            .order_by_desc(SalaryStructure::EFFECTIVE_FROM)
            .first(&mut *tx)
            .await
    }

    /// The structure of `employee` effective on `day`.
    pub async fn effective(
        db: &Db,
        employee: i64,
        day: NaiveDate,
    ) -> Result<Option<SalaryStructure>> {
        SalaryStructure::query()
            .where_(SalaryStructure::EMPLOYEE_ID.eq(employee))
            .where_(SalaryStructure::EFFECTIVE_FROM.lte(day))
            .order_by_desc(SalaryStructure::EFFECTIVE_FROM)
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structure() -> SalaryStructure {
        SalaryStructure {
            basic: 240_000,
            housing: 40_000,
            transport: 20_000,
            other_allowances: 0,
            ..SalaryStructure::default()
        }
    }

    #[test]
    fn the_fixed_wage_is_the_basic_wage_and_the_fixed_allowances() {
        assert_eq!(structure().fixed_wage(), 300_000);
    }

    #[test]
    fn the_hourly_rate_is_set_or_derived_from_the_basic_wage() {
        // 2 400.00 / (30 x 8) = 10.00 an hour.
        assert_eq!(structure().hourly(), 1_000);
        let set = SalaryStructure {
            hourly_rate: Some(1_500),
            ..structure()
        };
        assert_eq!(set.hourly(), 1_500);
    }
}
