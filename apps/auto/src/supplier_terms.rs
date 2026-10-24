//! Whether a supplier's warranty applies to a failed part (08 §7.1 *Supplier
//! warranty claim*, `AUT.WAR.02`).
//!
//! The warranty **applies** when the failure date is within the period from
//! the part's purchase date from the supplier and, when the terms set a
//! mileage and the part was fitted on a job card, the vehicle's mileage at the
//! failure less its mileage at fitting is within it; a part sold over the
//! counter is checked on the period only. *Labour included* does not decide
//! whether the warranty applies: it adds the labour of the replacement to the
//! claim amount ([`claim_amount`]).

use renox::chrono::NaiveDate;

use crate::fleet_service::add_months;

/// The warranty terms of a brand or a supplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terms {
    /// The period in months from the purchase date.
    pub months: i64,
    /// The mileage in kilometres from the fitting, when the terms set one.
    pub km: Option<i64>,
}

/// What is known of the failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// The day the part was bought from the supplier.
    pub purchase_date: NaiveDate,
    /// The day it failed (the warranty ticket's opening date).
    pub failure_date: NaiveDate,
    /// The vehicle's mileage when the part was fitted on a job card.
    pub fitted_odometer: Option<i64>,
    /// The vehicle's mileage at the failure.
    pub failure_odometer: Option<i64>,
}

/// The term the failure is out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutOfTerms {
    /// The failure came after the warranty period (or before the purchase).
    Period,
    /// The vehicle ran further than the terms allow since the fitting.
    Mileage,
    /// The terms set a mileage and the part was fitted on a job card, but the
    /// mileage at the failure is not given.
    MileageUnknown,
}

/// Whether the warranty applies; the term that failed when it does not.
///
/// # Errors
///
/// The [`OutOfTerms`] reason.
pub fn check(terms: Terms, facts: Facts) -> Result<(), OutOfTerms> {
    let last_day = add_months(facts.purchase_date, terms.months);
    if facts.failure_date < facts.purchase_date || facts.failure_date > last_day {
        return Err(OutOfTerms::Period);
    }
    if let (Some(limit), Some(fitted)) = (terms.km, facts.fitted_odometer) {
        let Some(failed_at) = facts.failure_odometer else {
            return Err(OutOfTerms::MileageUnknown);
        };
        if failed_at - fitted > limit {
            return Err(OutOfTerms::Mileage);
        }
    }
    Ok(())
}

/// What is claimed: the part, and the labour of the replacement when the
/// terms include it.
pub fn claim_amount(part_cost: i64, labour_included: bool, labour: i64) -> i64 {
    part_cost + if labour_included { labour.max(0) } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap()
    }

    fn facts(failure: NaiveDate, fitted: Option<i64>, failed_at: Option<i64>) -> Facts {
        Facts {
            purchase_date: day(5, 1),
            failure_date: failure,
            fitted_odometer: fitted,
            failure_odometer: failed_at,
        }
    }

    #[test]
    fn a_failure_inside_the_period_is_covered_through_its_last_day() {
        let terms = Terms {
            months: 12,
            km: None,
        };
        assert_eq!(check(terms, facts(day(11, 2), None, None)), Ok(()));
        let last = NaiveDate::from_ymd_opt(2027, 5, 1).unwrap();
        assert_eq!(check(terms, facts(last, None, None)), Ok(()));
        assert_eq!(
            check(terms, facts(last.succ_opt().unwrap(), None, None)),
            Err(OutOfTerms::Period)
        );
        assert_eq!(
            check(terms, facts(day(4, 30), None, None)),
            Err(OutOfTerms::Period),
            "not before the purchase"
        );
    }

    #[test]
    fn a_fitted_part_is_also_held_to_the_mileage_of_the_terms() {
        let terms = Terms {
            months: 12,
            km: Some(20_000),
        };
        assert_eq!(
            check(terms, facts(day(11, 2), Some(40_000), Some(58_000))),
            Ok(())
        );
        assert_eq!(
            check(terms, facts(day(11, 2), Some(40_000), Some(60_000))),
            Ok(()),
            "exactly the mileage is within it"
        );
        assert_eq!(
            check(terms, facts(day(11, 2), Some(40_000), Some(65_000))),
            Err(OutOfTerms::Mileage)
        );
        assert_eq!(
            check(terms, facts(day(11, 2), Some(40_000), None)),
            Err(OutOfTerms::MileageUnknown)
        );
    }

    #[test]
    fn a_part_sold_over_the_counter_is_checked_on_the_period_only() {
        let terms = Terms {
            months: 12,
            km: Some(20_000),
        };
        assert_eq!(check(terms, facts(day(11, 2), None, Some(900_000))), Ok(()));
    }

    #[test]
    fn labour_is_claimed_only_when_the_terms_include_it() {
        assert_eq!(claim_amount(30_000, false, 18_000), 30_000);
        assert_eq!(claim_amount(30_000, true, 18_000), 48_000);
        assert_eq!(claim_amount(30_000, true, -5), 30_000);
    }
}
