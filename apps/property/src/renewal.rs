//! The renewal window and the notice deadline (`PRP.RUL.01`, `PRP.RUL.09`,
//! 01 Property §2, §4.3 steps 1 and 2, §7.1 `flag_renewal`, §7.2; BL-PRP-03).
//!
//! In Dubai a change of rent or terms at renewal needs written notice at
//! least 90 days before the end date (Law No. 26 of 2007, Art. 14), or the
//! period the contract sets. The daily run ([`run`], 02:00 Asia/Dubai, also
//! the command [`TASK`]) does two things for the day it is given:
//!
//! - **`PRP.RUL.01`** ([`flag_due`]): an `active` lease enters the renewal
//!   window `max(120, contract notice period + 30)` days before its end date
//!   ([`window_opens_on`]): the `flag_renewal` transition, then the property
//!   managers and the tenant are told (a normal message). A lease activated
//!   late is flagged on the first run after its activation.
//! - **`PRP.RUL.09`**, compliance-critical ([`run_deadlines`]): for a
//!   `renewal_due` lease whose decision is `undecided`, or `changed_terms`
//!   without a timely proof of delivery, the property managers are warned 14
//!   and 3 days before the notice deadline ([`notice_deadline`]) and, the day
//!   after it, the lease gets the current-terms-only marker. The run is one
//!   rule run per date, so a failed or missed run alerts the owner role and
//!   the company admin (`PLT.RUL.04`).
//!
//! The same run goes on with the lease after its end date: `PRP.RUL.03`,
//! `PRP.RUL.14` and the extension of the charge schedule are in
//! [`crate::holding`].
//!
//! The notice of changed terms is a statutory notice that the property
//! manager sends ([`send_changed_terms`]); it counts as given only from its
//! delivery date, read from the notifier ([`delivered_on`]): the provider's
//! receipt, or the proof staff recorded when the message gave up. A sent but
//! undelivered notice is never counted. Nothing here sends the notice by
//! itself.
//!
//! Decided where the spec is silent:
//!
//! - A lease whose decision is `same_terms` or `not_renewing` gets neither
//!   warning nor marker.
//! - A warning is sent once per offset; when a run was missed and two offsets
//!   are due together only the nearest one is sent. A lease flagged after an
//!   offset date (activated late) gets no warning for that offset. A lease
//!   flagged by hand has no stored flag day and counts as flagged early.
//! - The marker is set once, with one message to the property managers, and
//!   a proof recorded later never clears it.
//! - The renewal decision can be set while the lease is `active` or
//!   `renewal_due`.

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::compliance::{CriticalRule, occurrence_key, schedule_subject};
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::notify::{NoticeStatus, notifier};

use crate::building::refused;
use crate::holding;
use crate::lease::{Lease, LeaseStatus, RenewalDecision};
use crate::lease_machine::{FLAG_RENEWAL, FlagInput};
use crate::notices::{self, NOTICE_OF_CHANGED_TERMS};
use crate::permissions;
use crate::texts::english;

/// The scheduled task and the command.
pub const TASK: &str = "property:renewals";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The rule that opens the renewal window.
pub const RULE_FLAG: &str = "PRP.RUL.01";

/// The compliance-critical rule that guards the notice deadline.
pub const RULE_DEADLINE: &str = "PRP.RUL.09";

/// N of `PRP.RUL.01`: the days before the end date the window opens, when the
/// contract's notice period needs no longer one.
pub const WINDOW_DAYS: i64 = 120;

/// The days between the opening of the window and the notice deadline that
/// the property manager has to decide in.
pub const DECISION_DAYS: i64 = 30;

/// N of `PRP.RUL.09`: the days before the deadline the property manager is
/// warned, first and second.
pub const WARN_DAYS: [i64; 2] = [14, 3];

/// The days before the end date the renewal window opens for `lease`:
/// `max(120, notice period + 30)`.
pub fn window_days(lease: &Lease) -> i64 {
    WINDOW_DAYS.max(lease.notice_days() + DECISION_DAYS)
}

/// The day `lease` enters the renewal window (`PRP.RUL.01`).
pub fn window_opens_on(lease: &Lease) -> NaiveDate {
    lease.end_date - Duration::days(window_days(lease))
}

/// The last day a notice of changed terms can be delivered for `lease`: the
/// end date less the notice period. A notice delivered on that day counts.
pub fn notice_deadline(lease: &Lease) -> NaiveDate {
    lease.end_date - Duration::days(lease.notice_days())
}

/// Whether the notice of changed terms of `lease` was delivered in time:
/// `delivered` is the delivery date, or `None` when it is not proved.
pub fn timely(lease: &Lease, delivered: Option<NaiveDate>) -> bool {
    delivered.is_some_and(|day| day <= notice_deadline(lease))
}

/// The delivery date of the notice of changed terms of `lease`, when it was
/// delivered: stored on the lease once known, else asked from the notifier,
/// which answers the provider's receipt or the proof staff recorded. A sent
/// notice without either is not delivered. What is found is stored on the
/// lease (the first date found stays).
///
/// # Errors
///
/// The notifier's and the database's errors.
pub async fn delivered_on(state: &AppState, lease: &Lease) -> Result<Option<NaiveDate>> {
    if let Some(day) = lease.notice_of_changes_delivered_on {
        return Ok(Some(day));
    }
    let status = notifier(state)
        .notice_status(state, &lease.subject(), NOTICE_OF_CHANGED_TERMS)
        .await?;
    let (day, proof) = match status {
        NoticeStatus::Delivered { at } => (dubai_date(at), "property.renewal.proof_receipt"),
        NoticeStatus::ProofRecorded { delivered_on } => {
            (delivered_on, "property.renewal.proof_recorded")
        }
        NoticeStatus::NotDelivered => return Ok(None),
    };
    Lease::query()
        .where_(Lease::ID.eq(lease.id))
        .where_(Lease::NOTICE_OF_CHANGES_DELIVERED_ON.is_null())
        .update(
            &state.db,
            &[
                ("notice_of_changes_delivered_on", &Some(day)),
                ("notice_of_changes_proof", &Some(english(proof, &[]))),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(Some(day))
}

/// The lease `id`, or a 404.
async fn load(state: &AppState, id: i64) -> Result<Lease> {
    Lease::find(&state.db, id).await?.ok_or(Error::NotFound)
}

/// 403 unless `user` holds `leases.manage` in the building of `lease`.
fn manager_only(user: &AuthUser, lease: &Lease) -> Result {
    if user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// `user` records the renewal decision on lease `id` (§4.3 step 1).
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 unless the lease
/// is `active` or `renewal_due`.
pub async fn set_decision(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    decision: RenewalDecision,
) -> Result<Lease> {
    let lease = load(state, id).await?;
    manager_only(user, &lease)?;
    if !matches!(lease.status, LeaseStatus::Active | LeaseStatus::RenewalDue) {
        return Err(refused(english("property.renewal.decision_refused", &[])));
    }
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[
                ("renewal_decision", &decision),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    load(state, id).await
}

/// `user` sends the statutory notice of changed terms of lease `id` to the
/// tenant, and the day it was sent is recorded. Its delivery is proved by the
/// notifier ([`delivered_on`]); a notice that gave up needs the proof staff
/// record.
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 unless the lease
/// is `active` or `renewal_due`, the decision is `changed_terms` and the
/// notice was not delivered yet.
pub async fn send_changed_terms(state: &AppState, user: &AuthUser, id: i64) -> Result<Lease> {
    let lease = load(state, id).await?;
    manager_only(user, &lease)?;
    if !matches!(lease.status, LeaseStatus::Active | LeaseStatus::RenewalDue) {
        return Err(refused(english("property.renewal.notice_refused", &[])));
    }
    if lease.renewal_decision != RenewalDecision::ChangedTerms {
        return Err(refused(english("property.renewal.notice_needs_decision", &[])));
    }
    if delivered_on(state, &lease).await?.is_some() {
        return Err(refused(english(
            "property.renewal.notice_already_delivered",
            &[],
        )));
    }
    notices::changed_terms(state, &lease, notice_deadline(&lease)).await?;
    Lease::query()
        .where_(Lease::ID.eq(id))
        .update(
            &state.db,
            &[
                ("notice_of_changes_sent_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    load(state, id).await
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Leases that entered the renewal window.
    pub flagged: usize,
    /// Deadline warnings sent.
    pub warned: usize,
    /// Leases that got the current-terms-only marker.
    pub marked: usize,
    /// Leases that expired (`PRP.RUL.03`).
    pub expired: usize,
    /// Statutory renewal periods added to the charge schedule of a lease
    /// that is still holding over.
    pub extended: usize,
    /// Holding-over tasks opened (`PRP.RUL.14`).
    pub tasked: usize,
}

/// `PRP.RUL.01` for `day`: flags the `active` leases whose window is open.
/// Every lease is tried; the first failure is returned after the others.
///
/// # Errors
///
/// The database's errors, or the first lease that could not be flagged.
pub async fn flag_due(state: &AppState, day: NaiveDate) -> Result<usize> {
    let active = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::Active))
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    let mut flagged = 0;
    let mut failure = None;
    for lease in active.iter().filter(|l| day >= window_opens_on(l)) {
        match lease
            .transition(FLAG_RENEWAL)
            .by_system()
            .input(&FlagInput { day: Some(day) })
            .run(state)
            .await
        {
            Ok(_) => flagged += 1,
            // Someone moved it first.
            Err(TransitionError::NotFrom(_) | TransitionError::Conflict) => {}
            Err(other) => {
                eprintln!("{TASK}: lease {}: {other}", lease.id);
                failure.get_or_insert_with(|| other.into_error());
            }
        }
    }
    failure.map_or(Ok(flagged), Err)
}

/// The text that says what is missing before the deadline.
fn problem_of(lease: &Lease) -> String {
    let key = match lease.renewal_decision {
        RenewalDecision::ChangedTerms => "property.renewal.problem_undelivered",
        _ => "property.renewal.problem_undecided",
    };
    english(key, &[])
}

/// `PRP.RUL.09` for one `renewal_due` lease on `day`.
async fn handle_deadline(
    state: &AppState,
    lease: &Lease,
    day: NaiveDate,
    report: &mut Report,
) -> Result {
    let delivered = match lease.renewal_decision {
        RenewalDecision::Undecided => None,
        RenewalDecision::ChangedTerms => delivered_on(state, lease).await?,
        // Nothing to give notice of.
        RenewalDecision::SameTerms | RenewalDecision::NotRenewing => return Ok(()),
    };
    if lease.renewal_decision == RenewalDecision::ChangedTerms && timely(lease, delivered) {
        return Ok(());
    }
    let deadline = notice_deadline(lease);
    if day > deadline {
        if !lease.current_terms_only {
            notices::current_terms_only(state, lease, deadline).await?;
            Lease::query()
                .where_(Lease::ID.eq(lease.id))
                .update(
                    &state.db,
                    &[
                        ("current_terms_only", &true),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            report.marked += 1;
        }
        return Ok(());
    }
    let flagged = lease.renewal_flagged_on.unwrap_or(NaiveDate::MIN);
    let columns = ["deadline_warned_first_on", "deadline_warned_second_on"];
    let sent = [lease.deadline_warned_first_on, lease.deadline_warned_second_on];
    let due: Vec<usize> = (0..WARN_DAYS.len())
        .filter(|&i| {
            let at = deadline - Duration::days(WARN_DAYS[i]);
            day >= at && flagged <= at && sent[i].is_none()
        })
        .collect();
    if due.is_empty() {
        return Ok(());
    }
    notices::renewal_deadline(
        state,
        lease,
        (deadline - day).num_days(),
        deadline,
        &problem_of(lease),
    )
    .await?;
    // Stamped after the message: a failed send is tried again by the retry.
    for index in due {
        Lease::query()
            .where_(Lease::ID.eq(lease.id))
            .update(
                &state.db,
                &[
                    (columns[index], &Some(day)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    report.warned += 1;
    Ok(())
}

/// The action of the `PRP.RUL.09` run: every `renewal_due` lease, each tried
/// even when another fails; the first failure is returned so the run is
/// retried (the steps are repeatable).
async fn deadlines(state: &AppState, day: NaiveDate, report: &mut Report) -> Result {
    let due = Lease::query()
        .where_(Lease::STATUS.eq(LeaseStatus::RenewalDue))
        .order_by(Lease::ID)
        .get(&state.db)
        .await?;
    let mut failure = None;
    for lease in &due {
        if let Err(error) = handle_deadline(state, lease, day, report).await {
            eprintln!("{TASK}: lease {}: {error:?}", lease.id);
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Runs `PRP.RUL.09` for `day` as one compliance-critical rule run: a second
/// run for the same date does nothing, a failed one is retried, and the
/// fifth failure alerts the owner role and the company admin.
///
/// # Errors
///
/// The database's errors.
pub async fn run_deadlines(state: &AppState, day: NaiveDate) -> Result<Report> {
    let spec = RuleRunSpec::new(RULE_DEADLINE, schedule_subject(), occurrence_key(RULE_DEADLINE, day))
        .compliance_critical();
    let seen = Arc::new(Mutex::new(Report::default()));
    let out = Arc::clone(&seen);
    run_rule(state, spec, move |state: AppState| async move {
        let mut report = Report::default();
        let result = deadlines(&state, day, &mut report).await;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = report;
        result
    })
    .await?;
    let report = *seen.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(report)
}

/// Runs the daily rules for `day` (Asia/Dubai) in the order of a lease's
/// life: the renewal window (`PRP.RUL.01`), the notice deadline
/// (`PRP.RUL.09`), expiry after the end date (`PRP.RUL.03`), the next period
/// of a lease still holding over, and the holding-over tasks
/// (`PRP.RUL.14`). A failing step does not stop the others.
///
/// # Errors
///
/// The first error of the steps.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let window = flag_due(state, day).await;
    let guard = run_deadlines(state, day).await;
    let expired = holding::expire_due(state, day).await;
    let extended = holding::extend_due(state, day).await;
    let tasked = holding::open_tasks(state, day).await;
    let mut report = guard?;
    report.flagged = window?;
    report.expired = expired?;
    report.extended = extended?;
    report.tasked = tasked?;
    Ok(report)
}

/// Marks `PRP.RUL.09` compliance-critical and registers the daily task and
/// its command.
pub(crate) fn schedule(app: &mut Registry) {
    CriticalRule::new(RULE_DEADLINE).daily_at(TASK_AT).register();
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Open the renewal window, guard the notice deadline and handle the leases held over (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} leases flagged, {} warnings, {} marked current terms only, {} expired, {} periods added, {} tasks opened.",
                report.flagged,
                report.warned,
                report.marked,
                report.expired,
                report.extended,
                report.tasked
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn lease(end: NaiveDate, notice: Option<i32>) -> Lease {
        Lease {
            end_date: end,
            contract_notice_days: notice,
            ..Default::default()
        }
    }

    #[test]
    fn the_window_opens_120_days_and_the_deadline_is_90_days_before_the_end() {
        let lease = lease(day(2027, 12, 31), None);
        assert_eq!(window_opens_on(&lease), day(2027, 9, 2));
        assert_eq!(notice_deadline(&lease), day(2027, 10, 2));
    }

    #[test]
    fn a_longer_contract_notice_opens_the_window_a_month_before_its_deadline() {
        let lease = lease(day(2027, 12, 31), Some(120));
        assert_eq!(window_days(&lease), 150);
        assert_eq!(window_opens_on(&lease), day(2027, 8, 3));
        assert_eq!(notice_deadline(&lease), day(2027, 9, 2));
    }

    #[test]
    fn a_shorter_contract_notice_moves_the_deadline_and_keeps_the_window() {
        let lease = lease(day(2027, 12, 31), Some(60));
        assert_eq!(window_days(&lease), 120);
        assert_eq!(notice_deadline(&lease), day(2027, 11, 1));
    }

    #[test]
    fn a_notice_delivered_on_the_deadline_day_is_timely() {
        let lease = lease(day(2027, 12, 31), None);
        assert!(timely(&lease, Some(day(2027, 10, 2))));
        assert!(timely(&lease, Some(day(2027, 9, 20))));
        assert!(!timely(&lease, Some(day(2027, 10, 3))));
        assert!(!timely(&lease, None));
    }

    #[test]
    fn the_warnings_fall_14_and_3_days_before_the_deadline() {
        let deadline = notice_deadline(&lease(day(2027, 12, 31), None));
        let days: Vec<_> = WARN_DAYS
            .iter()
            .map(|n| deadline - Duration::days(*n))
            .collect();
        assert_eq!(days, [day(2027, 9, 18), day(2027, 9, 29)]);
    }
}
