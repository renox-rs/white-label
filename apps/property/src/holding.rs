//! Holding over after the end date (`expire`, `PRP.RUL.03`, `PRP.RUL.14`,
//! 01 Property §2 Art. 6, §4.3, §7.1 `expire`; BL-PRP-06).
//!
//! When a `renewal_due` lease passes its end date with no notice and no
//! active renewal version it becomes `expired`. That is not an end: in Dubai
//! the tenant who stays is renewed on the same terms (Law No. 26 of 2007,
//! Art. 6), so billing continues.
//!
//! - **`PRP.RUL.03`** ([`expire_due`]): the day after the end date the
//!   `expire` transition runs. Its effect extends the charge schedule on the
//!   same terms for the *statutory renewal period*, the lease's own period or
//!   one year, whichever is shorter ([`period_end`], [`plan_extension`]), and
//!   stores the end of the period as "holding over until"; the property
//!   managers are told after the commit. The lease's end date, the charges
//!   already scheduled and their amounts are not changed.
//! - **Each period** ([`extend_due`]): a lease still `expired` the day after
//!   "holding over until" gets the schedule extended again, once per period.
//! - **`PRP.RUL.14`** ([`open_tasks`]): 7 days after the lease became
//!   `expired`, and at the start of each later period, the property manager
//!   gets a task to issue the renewal version on the same terms and renew
//!   Ejari, or record a notice. A renewal version in `draft` or
//!   `pending_signature` stops a new task from being created; an open task
//!   closes only when a renewal version is submitted (`pending_signature`),
//!   not when a draft is created ([`renewal_submitted`]). One task per period.
//!
//! The extension repeats the payment plan of the version's own term: the
//! charges due inside it, with their types, amounts and VAT categories,
//! shifted forward to the period (by whole months when the term is whole
//! months, else by days). A 1-year lease with 12 monthly charges gets 12
//! more; a 2-year lease gets one year of its plan; a 6-month lease gets 6
//! months. No cheque is linked to the new charges; they are invoiced on their
//! due dates by the daily run like any other.
//!
//! Decided where the spec is silent:
//!
//! - Abu Dhabi behaves as Dubai until the lawyer answers L-22; there is no
//!   emirate-specific logic.
//! - A landlord who objects records a notice; there is no objection guard.
//! - The extension charges are marked [`LeaseCharge::holding_over`] so the
//!   invoices made from them can be told (BL-PRP-07).

use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::tickets::task_creator;

use crate::charge::{ChargeStatus, LeaseCharge};
use crate::charge_type::EARLY_TERMINATION_FEE;
use crate::db::{refused, set_lease};
use crate::lease::{Lease, LeaseStatus, VersionKind};
use crate::lease_machine::{EXPIRE, ExpireInput};
use crate::notices;
use crate::permissions::PROPERTY_MANAGER_ROLE;
use crate::texts::english;

/// N of `PRP.RUL.14`: the days after the lease became `expired` that the
/// first holding-over task is opened.
pub const TASK_AFTER: i64 = 7;

/// The whole months from `from` to `to`, when `to` is exactly that many
/// months after `from` (`2027-01-01` to `2028-01-01` is 12).
pub fn whole_months(from: NaiveDate, to: NaiveDate) -> Option<u32> {
    let months = (to.year() - from.year()) * 12 + i32::try_from(to.month()).ok()?
        - i32::try_from(from.month()).ok()?;
    let months = u32::try_from(months).ok().filter(|m| *m > 0)?;
    (from.checked_add_months(Months::new(months)) == Some(to)).then_some(months)
}

/// The last day of the statutory renewal period that starts on `from`: the
/// length of the term of `lease` or one year, whichever is shorter. A term of
/// whole months gives the same months, any other term the same days.
pub fn period_end(lease: &Lease, from: NaiveDate) -> NaiveDate {
    let year_end = from
        .checked_add_months(Months::new(12))
        .map_or(from + Duration::days(364), |next| next - Duration::days(1));
    let own = whole_months(lease.start_date, lease.end_date + Duration::days(1))
        .and_then(|months| from.checked_add_months(Months::new(months)))
        .map_or_else(
            || from + Duration::days((lease.end_date - lease.start_date).num_days()),
            |next| next - Duration::days(1),
        );
    own.min(year_end)
}

/// How the due dates of the plan move to a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shift {
    /// By whole months, so a month end does not drift.
    Months(u32),
    /// By days, for a term that is not whole months.
    Days(i64),
}

impl Shift {
    /// From the start of the term to the start of the period.
    fn between(start: NaiveDate, period: NaiveDate) -> Self {
        whole_months(start, period).map_or_else(
            || Shift::Days((period - start).num_days()),
            Shift::Months,
        )
    }

    fn apply(self, due: NaiveDate) -> Option<NaiveDate> {
        match self {
            Shift::Months(months) => due.checked_add_months(Months::new(months)),
            Shift::Days(days) => due.checked_add_signed(Duration::days(days)),
        }
    }
}

/// One charge of an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionLine {
    /// Its place in the plan, after the existing charges.
    pub position: i32,
    /// The charge type of the charge it repeats.
    pub charge_type: String,
    /// The invoice line text.
    pub description: String,
    /// The due date, in the period.
    pub due_date: NaiveDate,
    /// The amount before VAT, in fils.
    pub amount: i64,
    /// The VAT treatment.
    pub vat_category: wl_kernel::VatCategory,
}

/// The extension of the charge schedule for the period that starts on `from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    /// The last day of the period: "holding over until".
    pub until: NaiveDate,
    /// The charges to create.
    pub lines: Vec<ExtensionLine>,
}

/// Plans the extension of `lease` for the period that starts on `from`, from
/// its `charges`: the live charges due inside its own term are repeated, moved
/// to the period, those falling after the period left out. Charges an earlier
/// extension created are not repeated, and a charge is not made twice for a
/// type and due date that a live charge has already (so a plan that already
/// runs past the end date is not billed twice).
pub fn plan_extension(lease: &Lease, charges: &[LeaseCharge], from: NaiveDate) -> Extension {
    let until = period_end(lease, from);
    let shift = Shift::between(lease.start_date, from);
    let mut base: Vec<&LeaseCharge> = charges
        .iter()
        .filter(|c| {
            !c.holding_over
                && c.status != ChargeStatus::Cancelled
                && c.due_date <= lease.end_date
                // A one-off penalty is not part of the plan that is repeated.
                && c.charge_type != EARLY_TERMINATION_FEE
        })
        .collect();
    base.sort_by_key(|c| (c.due_date, c.position, c.id));
    let taken: std::collections::BTreeSet<(&str, NaiveDate)> = charges
        .iter()
        .filter(|c| c.status != ChargeStatus::Cancelled)
        .map(|c| (c.charge_type.as_str(), c.due_date))
        .collect();
    let mut position = charges.iter().map(|c| c.position).max().unwrap_or(0);
    let lines = base
        .into_iter()
        .filter_map(|c| {
            let due = shift.apply(c.due_date).filter(|due| *due <= until)?;
            if taken.contains(&(c.charge_type.as_str(), due)) {
                return None;
            }
            position += 1;
            Some(ExtensionLine {
                position,
                charge_type: c.charge_type.clone(),
                description: english(
                    "property.holding.line",
                    &[("description", &c.description)],
                ),
                due_date: due,
                amount: c.amount,
                vat_category: c.vat_category,
            })
        })
        .collect();
    Extension { until, lines }
}

/// Creates the charges of the extension of `lease` that starts on `from`, as
/// scheduled (billing continues). Returns the end of the period.
async fn insert_lines(tx: &mut Transaction, lease: &Lease, from: NaiveDate) -> Result<NaiveDate> {
    let charges = LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(lease.id))
        .order_by(LeaseCharge::POSITION)
        .order_by(LeaseCharge::ID)
        .get(&mut *tx)
        .await?;
    let plan = plan_extension(lease, &charges, from);
    for line in plan.lines {
        LeaseCharge::create(
            &mut *tx,
            LeaseCharge {
                lease_id: lease.id,
                position: line.position,
                charge_type: line.charge_type,
                description: line.description,
                due_date: line.due_date,
                amount: line.amount,
                vat_category: line.vat_category,
                status: ChargeStatus::Scheduled,
                holding_over: true,
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(plan.until)
}

/// The effects of `expire` inside its transaction: the first period's charges,
/// "holding over until", and the day it expired. Refused when a renewal
/// version is `active` (it replaces this version at its own activation).
pub(crate) async fn expire_effects(tx: &mut Transaction, lease: &Lease, day: NaiveDate) -> Result {
    let replaced = Lease::query()
        .where_(Lease::PREVIOUS_ID.eq(lease.id))
        .where_(Lease::VERSION_KIND.eq(VersionKind::Renewal))
        .where_(Lease::STATUS.eq(LeaseStatus::Active))
        .first(&mut *tx)
        .await?;
    if replaced.is_some() {
        return Err(refused(english("property.holding.renewal_active", &[])));
    }
    let from = lease.end_date + Duration::days(1);
    let until = insert_lines(&mut *tx, lease, from).await?;
    set_lease(
        &mut *tx,
        lease.id,
        &[
            ("expired_on", &Some(day)),
            ("holding_over_until", &Some(until)),
            ("holding_period_start", &Some(from)),
        ],
    )
    .await
}

/// `PRP.RUL.03` for `day`: expires the `renewal_due` leases whose end date has
/// passed. Every lease is tried; the first failure is returned after the
/// others.
///
/// # Errors
///
/// The database's errors, or the first lease that could not be expired.
pub async fn expire_due(state: &AppState, day: NaiveDate) -> Result<usize> {
    let due = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::RenewalDue))
        .where_(Lease::HANDED_OVER_ON.is_null())
        .where_(Lease::END_DATE.lt(day))
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    let mut expired = 0;
    let mut failure = None;
    for lease in &due {
        match lease
            .transition(EXPIRE)
            .by_system()
            .input(&ExpireInput { day: Some(day) })
            .run(state)
            .await
        {
            Ok(_) => expired += 1,
            // Someone moved it first.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => {
                eprintln!("property:renewals: lease {}: {other}", lease.id);
                failure.get_or_insert_with(|| other.into_error());
            }
        }
    }
    failure.map_or(Ok(expired), Err)
}

/// Extends the schedule of `lease` by one period, once: `false` when another
/// run did it first or the lease is no longer holding over.
async fn extend_period(state: &AppState, lease: &Lease) -> Result<bool> {
    let Some(until) = lease.holding_over_until else {
        return Ok(false);
    };
    let from = until + Duration::days(1);
    let mut tx = state.db.begin().await?;
    // Claimed first: of two runs holding the same row, one extends.
    let claimed = Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .where_(Lease::STATUS.eq(LeaseStatus::Expired))
        .where_(Lease::HOLDING_OVER_UNTIL.eq(until))
        .update(
            &mut tx,
            &[
                ("holding_over_until", &Some(period_end(lease, from))),
                ("holding_period_start", &Some(from)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(false);
    }
    insert_lines(&mut tx, lease, from).await?;
    tx.commit().await?;
    Ok(true)
}

/// Extends the schedule of the `expired` leases whose period ended before
/// `day`, once per period. Returns how many periods were added.
///
/// # Errors
///
/// The database's errors, or the first lease that could not be extended.
pub async fn extend_due(state: &AppState, day: NaiveDate) -> Result<usize> {
    let holding = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::Expired))
        .where_(Lease::HANDED_OVER_ON.is_null())
        .where_(Lease::HOLDING_OVER_UNTIL.lt(day))
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    let mut extended = 0;
    let mut failure = None;
    for first in holding {
        let mut lease = first;
        // A run that was missed for a long time catches up period by period.
        for _ in 0..50 {
            if lease.holding_over_until.is_none_or(|until| until >= day) {
                break;
            }
            match extend_period(state, &lease).await {
                Ok(true) => extended += 1,
                Ok(false) => break,
                Err(error) => {
                    eprintln!("property:renewals: lease {}: {error:?}", lease.id);
                    failure.get_or_insert(error);
                    break;
                }
            }
            match Lease::find(&state.db, lease.id).await? {
                Some(fresh) => lease = fresh,
                None => break,
            }
        }
    }
    failure.map_or(Ok(extended), Err)
}

/// The key the holding-over task of `lease` is found by while it is open,
/// for the period it was opened for.
pub fn task_key(lease: &Lease, period_start: NaiveDate) -> String {
    format!("property.holding:{}:{period_start}", lease.id)
}

/// The day the task of the period that starts on `period_start` is due: 7
/// days after the lease became `expired` for the first period, the start of
/// the period for the later ones.
pub fn task_due_on(lease: &Lease, period_start: NaiveDate) -> NaiveDate {
    if period_start == lease.end_date + Duration::days(1) {
        lease.expired_on.unwrap_or(period_start) + Duration::days(TASK_AFTER)
    } else {
        period_start
    }
}

/// Closes the open holding-over task of `lease`, if there is one.
async fn close_task(state: &AppState, lease: &Lease) -> Result {
    let Some(period) = lease.holding_period_start else {
        return Ok(());
    };
    let creator = task_creator(state);
    if let Some(task) = creator.find_open(state, &task_key(lease, period)).await? {
        creator
            .complete(state, task.id, &english("property.holding.task_done", &[]))
            .await?;
    }
    Ok(())
}

/// The renewal versions of `lease` still being prepared or signed.
async fn renewals_in_progress(state: &AppState, lease: &Lease) -> Result<Vec<Lease>> {
    Ok(Lease::follow_ups_in_progress(&state.db, lease.root_id)
        .await?
        .into_iter()
        .filter(|l| l.version_kind == VersionKind::Renewal && l.previous_id == Some(lease.id))
        .collect())
}

/// A renewal version was submitted: the open holding-over task of the version
/// it renews closes (`PRP.RUL.14`). A draft does not close it.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub async fn renewal_submitted(state: &AppState, id: i64) -> Result {
    let Some(version) = Lease::find(&state.db, id).await? else {
        return Ok(());
    };
    if version.version_kind != VersionKind::Renewal {
        return Ok(());
    }
    if let Some(previous) = version.previous(&state.db).await?
        && previous.status == LeaseStatus::Expired
    {
        close_task(state, &previous).await?;
    }
    Ok(())
}

/// `PRP.RUL.14` for one `expired` lease on `day`.
async fn handle_task(state: &AppState, lease: &Lease, day: NaiveDate) -> Result<bool> {
    let Some(period) = lease.holding_period_start else {
        return Ok(false);
    };
    let renewals = renewals_in_progress(state, lease).await?;
    // The safety net of the listener: a submitted version closes the task.
    if renewals
        .iter()
        .any(|l| l.status == LeaseStatus::PendingSignature)
    {
        close_task(state, lease).await?;
    }
    // A draft or a submitted version is enough to stop a new task.
    if !renewals.is_empty() {
        return Ok(false);
    }
    if day < task_due_on(lease, period) || lease.holding_tasked_for == Some(period) {
        return Ok(false);
    }
    // Stamped first: a task closed by the manager is not opened again.
    Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .update(
            &state.db,
            &[
                ("holding_tasked_for", &Some(period)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let until = lease.holding_over_until.unwrap_or(period);
    let task = RoleTask::new(
        lease.subject(),
        PROPERTY_MANAGER_ROLE,
        &english(
            "property.holding.task_title",
            &[("lease", &lease.number_with_version())],
        ),
        &task_key(lease, period),
    )
    .description(&english(
        "property.holding.task_description",
        &[("until", &until)],
    ));
    crate::tasks::open(state, &lease.scope(), task).await?;
    Ok(true)
}

/// `PRP.RUL.14` for `day`: opens the holding-over tasks that are due and
/// closes those whose renewal version was submitted. Returns how many were
/// opened.
///
/// # Errors
///
/// The database's errors, or the first lease that could not be handled.
pub async fn open_tasks(state: &AppState, day: NaiveDate) -> Result<usize> {
    let expired = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::Expired))
        .where_(Lease::HANDED_OVER_ON.is_null())
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    let mut opened = 0;
    let mut failure = None;
    for lease in &expired {
        match handle_task(state, lease, day).await {
            Ok(true) => opened += 1,
            Ok(false) => {}
            Err(error) => {
                eprintln!("property:renewals: lease {}: {error:?}", lease.id);
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(opened), Err)
}

/// Tells the property managers that `lease` is holding over (after the commit
/// of `expire`).
///
/// # Errors
///
/// The notifier's errors.
pub async fn announce(state: &AppState, id: i64) -> Result {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    notices::holding_over(state, &lease).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_kernel::VatCategory;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn lease(start: NaiveDate, end: NaiveDate) -> Lease {
        Lease {
            start_date: start,
            end_date: end,
            ..Default::default()
        }
    }

    fn monthly(first: NaiveDate, months: u32, amount: i64) -> Vec<LeaseCharge> {
        (0..months)
            .map(|i| LeaseCharge {
                id: i64::from(i) + 1,
                position: i32::try_from(i).unwrap() + 1,
                charge_type: "rent".into(),
                description: format!("Rent {}/{months}", i + 1),
                due_date: first.checked_add_months(Months::new(i)).unwrap(),
                amount,
                vat_category: VatCategory::Exempt,
                status: ChargeStatus::Scheduled,
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn whole_months_are_counted_only_when_exact() {
        assert_eq!(whole_months(day(2027, 1, 1), day(2028, 1, 1)), Some(12));
        assert_eq!(whole_months(day(2027, 1, 1), day(2027, 7, 1)), Some(6));
        assert_eq!(whole_months(day(2027, 1, 1), day(2027, 7, 2)), None);
        assert_eq!(whole_months(day(2027, 1, 1), day(2027, 1, 1)), None);
        assert_eq!(whole_months(day(2027, 6, 1), day(2027, 7, 31)), None);
    }

    #[test]
    fn the_period_is_the_lease_term_or_one_year() {
        let year = lease(day(2027, 1, 1), day(2027, 12, 31));
        assert_eq!(period_end(&year, day(2028, 1, 1)), day(2028, 12, 31));
        assert_eq!(period_end(&year, day(2029, 1, 1)), day(2029, 12, 31));
        let two_years = lease(day(2026, 1, 1), day(2027, 12, 31));
        assert_eq!(period_end(&two_years, day(2028, 1, 1)), day(2028, 12, 31));
        let half = lease(day(2027, 1, 1), day(2027, 6, 30));
        assert_eq!(period_end(&half, day(2027, 7, 1)), day(2027, 12, 31));
        assert_eq!(period_end(&half, day(2028, 1, 1)), day(2028, 6, 30));
        // 60 days: not whole months, so the same days.
        let sixty = lease(day(2027, 6, 1), day(2027, 7, 30));
        assert_eq!(period_end(&sixty, day(2027, 7, 31)), day(2027, 9, 28));
    }

    #[test]
    fn a_year_of_monthly_rent_is_repeated_for_the_next_year() {
        let year = lease(day(2027, 1, 1), day(2027, 12, 31));
        let charges = monthly(day(2027, 1, 1), 12, 1_000_000);
        let plan = plan_extension(&year, &charges, day(2028, 1, 1));
        assert_eq!(plan.until, day(2028, 12, 31));
        assert_eq!(plan.lines.len(), 12);
        assert_eq!(plan.lines[0].due_date, day(2028, 1, 1));
        assert_eq!(plan.lines[11].due_date, day(2028, 12, 1));
        assert!(plan.lines.iter().all(|l| l.amount == 1_000_000));
        assert_eq!(plan.lines[0].position, 13);
        assert_eq!(plan.lines[0].description, "Rent 1/12 (holding over)");
    }

    #[test]
    fn a_two_year_lease_gets_one_year_of_its_plan() {
        let two = lease(day(2026, 1, 1), day(2027, 12, 31));
        let charges = monthly(day(2026, 1, 1), 24, 500_000);
        let plan = plan_extension(&two, &charges, day(2028, 1, 1));
        assert_eq!(plan.until, day(2028, 12, 31));
        assert_eq!(plan.lines.len(), 12);
        assert_eq!(plan.lines[0].due_date, day(2028, 1, 1));
        assert_eq!(plan.lines[11].due_date, day(2028, 12, 1));
    }

    #[test]
    fn a_six_month_lease_gets_six_months_each_time() {
        let half = lease(day(2027, 1, 1), day(2027, 6, 30));
        let mut charges = monthly(day(2027, 1, 1), 6, 100_000);
        let first = plan_extension(&half, &charges, day(2027, 7, 1));
        assert_eq!(first.lines.len(), 6);
        assert_eq!(first.lines[0].due_date, day(2027, 7, 1));
        assert_eq!(first.lines[5].due_date, day(2027, 12, 1));
        // The second period repeats the term's plan, not the first extension.
        for (i, line) in first.lines.iter().enumerate() {
            charges.push(LeaseCharge {
                id: 100 + i64::try_from(i).unwrap(),
                position: line.position,
                charge_type: line.charge_type.clone(),
                due_date: line.due_date,
                amount: line.amount,
                holding_over: true,
                status: ChargeStatus::Scheduled,
                ..Default::default()
            });
        }
        let second = plan_extension(&half, &charges, day(2028, 1, 1));
        assert_eq!(second.until, day(2028, 6, 30));
        assert_eq!(second.lines.len(), 6);
        assert_eq!(second.lines[0].due_date, day(2028, 1, 1));
        assert_eq!(second.lines[0].position, 13);
    }

    #[test]
    fn cancelled_charges_and_the_vat_of_the_plan_are_respected() {
        let year = lease(day(2027, 1, 1), day(2027, 12, 31));
        let mut charges = monthly(day(2027, 1, 1), 4, 2_500_000);
        charges[1].status = ChargeStatus::Cancelled;
        charges[2].vat_category = VatCategory::Standard;
        let plan = plan_extension(&year, &charges, day(2028, 1, 1));
        assert_eq!(plan.lines.len(), 3);
        assert_eq!(plan.lines[1].vat_category, VatCategory::Standard);
    }

    #[test]
    fn a_charge_that_exists_already_is_not_made_twice() {
        // A plan of 12 monthly charges on a 6-month lease already runs past
        // the end date: the months it covers are not extended.
        let half = lease(day(2027, 1, 1), day(2027, 6, 30));
        let charges = monthly(day(2027, 1, 1), 12, 100_000);
        let plan = plan_extension(&half, &charges, day(2027, 7, 1));
        assert!(plan.lines.is_empty());
        // The same plan asked for twice makes the same charges once.
        let year = lease(day(2027, 1, 1), day(2027, 12, 31));
        let mut all = monthly(day(2027, 1, 1), 12, 100_000);
        let first = plan_extension(&year, &all, day(2028, 1, 1));
        for (i, line) in first.lines.iter().enumerate() {
            all.push(LeaseCharge {
                id: 50 + i64::try_from(i).unwrap(),
                position: line.position,
                charge_type: line.charge_type.clone(),
                due_date: line.due_date,
                holding_over: true,
                status: ChargeStatus::Scheduled,
                ..Default::default()
            });
        }
        assert!(plan_extension(&year, &all, day(2028, 1, 1)).lines.is_empty());
    }

    #[test]
    fn the_first_task_is_due_seven_days_after_expiry_and_later_ones_at_the_period() {
        let lease = Lease {
            end_date: day(2027, 12, 31),
            expired_on: Some(day(2028, 1, 1)),
            ..Default::default()
        };
        assert_eq!(task_due_on(&lease, day(2028, 1, 1)), day(2028, 1, 8));
        assert_eq!(task_due_on(&lease, day(2029, 1, 1)), day(2029, 1, 1));
        assert_eq!(
            task_key(&Lease { id: 4, ..Default::default() }, day(2028, 1, 1)),
            "property.holding:4:2028-01-01"
        );
    }
}
