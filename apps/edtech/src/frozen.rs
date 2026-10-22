//! What follows a freeze or an unfreeze once it committed (10 §7.1 *Enrolment*
//! `freeze`, `unfreeze`, BL-EDU-04, BL-EDU-05): the package is given back the
//! days it stood frozen, and the membership follows (`EDU.RUL.14`, added with
//! BL-EDU-05).
//!
//! The days of a package are given back when the last cause ends: from the
//! day the enrolment became `frozen` ([`Enrolment::frozen_on`]) to the day it
//! is `active` again, whatever the causes that held it meanwhile, so two causes
//! that overlap give the days once. A student who unfreezes early gets the days
//! it was frozen, not the days it was planned for. The extension is the
//! system's `extend` of wl-sales (`PackageLedger::extend_days`): it is not an
//! extension the limit counts.
//!
//! A step that fails does not undo the unfreeze; it is reported on stderr, as
//! the money steps of a withdrawal are.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::sales::{PackageId, package_ledger};

use crate::enrolment::Enrolment;
use crate::texts::english;

/// The days `enrolment` stood frozen when it is active again on `on`.
pub fn frozen_days(enrolment: &Enrolment, on: NaiveDate) -> i64 {
    enrolment
        .frozen_on
        .map_or(0, |from| (on - from).num_days().max(0))
}

/// Gives the package of `enrolment` the days it stood frozen. Answers the
/// package's new last day; `None` when nothing moved.
///
/// # Errors
///
/// The ledger's errors.
pub async fn give_back_days(
    state: &AppState,
    enrolment: &Enrolment,
    on: NaiveDate,
) -> Result<Option<NaiveDate>> {
    let Some(package) = enrolment.package_id else {
        return Ok(None);
    };
    let days = frozen_days(enrolment, on);
    if days < 1 {
        return Ok(None);
    }
    let why = english(
        "edtech.frozen.package_reason",
        &[("enrolment", &enrolment.number()), ("days", &days)],
    );
    package_ledger(state)
        .extend_days(state, PackageId(package), days, &why)
        .await
}

/// What follows an unfreeze on `on`: the package gets the days back.
pub(crate) async fn after_unfreeze(state: &AppState, enrolment: &Enrolment, on: NaiveDate) {
    if let Err(error) = give_back_days(state, enrolment, on).await {
        eprintln!(
            "edtech: the package of {} is not given back its frozen days: {error:?}",
            enrolment.number()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, date: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, date).unwrap()
    }

    #[test]
    fn the_days_run_from_the_freeze_to_the_day_it_is_active_again() {
        let frozen = Enrolment {
            frozen_on: Some(day(11, 10)),
            ..Enrolment::default()
        };
        assert_eq!(frozen_days(&frozen, day(11, 20)), 10);
        assert_eq!(frozen_days(&frozen, day(11, 10)), 0);
        assert_eq!(frozen_days(&Enrolment::default(), day(11, 20)), 0);
    }
}
