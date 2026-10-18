//! Tax periods (wl-invoicing §7): the span a VAT summary covers. The FTA
//! assigns each registrant monthly or quarterly periods, quarterly ones in
//! one of three staggers.

use renox::chrono::{Datelike, Days, Months, NaiveDate};

/// How an entity's VAT periods are cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxPeriod {
    /// One period per calendar month.
    Monthly,
    /// Three-month periods, one starting in `first_month` (1 to 12; only
    /// its place in the quarter counts, so 4 and 7 are the same stagger
    /// as 1).
    Quarterly {
        /// A month a period starts in.
        first_month: u32,
    },
}

impl Default for TaxPeriod {
    /// Quarterly on calendar quarters (January to March, April to June, …).
    fn default() -> Self {
        TaxPeriod::Quarterly { first_month: 1 }
    }
}

/// The first and last day of the tax period of `period` that contains
/// `date`.
pub fn containing(period: TaxPeriod, date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let (months_back, length) = match period {
        TaxPeriod::Monthly => (0, 1),
        TaxPeriod::Quarterly { first_month } => {
            // Months since the stagger's quarter start, 0 to 2.
            let first = first_month.clamp(1, 12) - 1;
            ((date.month0() + 12 - first % 3) % 3, 3)
        }
    };
    let month_start = date.with_day(1).expect("day 1 exists");
    let start = month_start
        .checked_sub_months(Months::new(months_back))
        .expect("a date in range");
    let end = start
        .checked_add_months(Months::new(length))
        .and_then(|next| next.checked_sub_days(Days::new(1)))
        .expect("a date in range");
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn the_default_is_calendar_quarters() {
        let p = TaxPeriod::default();
        assert_eq!(
            containing(p, d(2026, 3, 15)),
            (d(2026, 1, 1), d(2026, 3, 31))
        );
        assert_eq!(
            containing(p, d(2026, 4, 1)),
            (d(2026, 4, 1), d(2026, 6, 30))
        );
        assert_eq!(
            containing(p, d(2026, 12, 31)),
            (d(2026, 10, 1), d(2026, 12, 31))
        );
    }

    #[test]
    fn a_month_is_its_own_period() {
        let p = TaxPeriod::Monthly;
        assert_eq!(
            containing(p, d(2026, 2, 10)),
            (d(2026, 2, 1), d(2026, 2, 28))
        );
        assert_eq!(
            containing(p, d(2028, 2, 29)),
            (d(2028, 2, 1), d(2028, 2, 29))
        );
    }

    #[test]
    fn staggered_quarters_cross_the_year() {
        let p = TaxPeriod::Quarterly { first_month: 2 };
        assert_eq!(
            containing(p, d(2026, 1, 20)),
            (d(2025, 11, 1), d(2026, 1, 31))
        );
        assert_eq!(
            containing(p, d(2026, 2, 1)),
            (d(2026, 2, 1), d(2026, 4, 30))
        );
        assert_eq!(
            containing(p, d(2026, 4, 30)),
            (d(2026, 2, 1), d(2026, 4, 30))
        );
        let p = TaxPeriod::Quarterly { first_month: 3 };
        assert_eq!(
            containing(p, d(2026, 2, 28)),
            (d(2025, 12, 1), d(2026, 2, 28))
        );
        assert_eq!(
            containing(p, d(2026, 3, 1)),
            (d(2026, 3, 1), d(2026, 5, 31))
        );
        // 5 is the same stagger as 2.
        let p = TaxPeriod::Quarterly { first_month: 5 };
        assert_eq!(
            containing(p, d(2026, 1, 20)),
            (d(2025, 11, 1), d(2026, 1, 31))
        );
    }

    #[test]
    fn every_day_is_in_exactly_one_period() {
        for p in [
            TaxPeriod::Monthly,
            TaxPeriod::Quarterly { first_month: 1 },
            TaxPeriod::Quarterly { first_month: 2 },
            TaxPeriod::Quarterly { first_month: 3 },
        ] {
            let mut day = d(2026, 1, 1);
            while day < d(2027, 3, 1) {
                let (start, end) = containing(p, day);
                assert!(start <= day && day <= end);
                assert_eq!(containing(p, start).0, start);
                assert_eq!(containing(p, end), (start, end));
                day = day.succ_opt().unwrap();
            }
        }
    }
}
