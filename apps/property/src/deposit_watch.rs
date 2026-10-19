//! The window to dispute the deposit deductions, and the escalation of a
//! dispute nobody answers (`PRP.RUL.11`, 01 Property §4.3 step 4 and row "The
//! tenant disputes the deductions", §7.1 *Deposit*; BL-PRP-16).
//!
//! The deductions of a lease's deposit are published to the tenant in numbered
//! versions; the tenant has N days (14) to dispute. wl-payments emits what
//! happens to them (`DepositDeductionsPublished`, `DepositDeductionsDisputed`,
//! `DepositDisputeAnswered`, `DepositSettled`); the vertical keeps one
//! [`DepositWatch`] per deposit of a lease from these events, and the daily run
//! ([`run`], 02:00 Asia/Dubai, also the command [`TASK`]) does two things:
//!
//! - **No dispute:** N days after the latest published version, while the
//!   settlement is pending and no line is disputed, the accountant is told
//!   that the settlement may proceed, once per published version. A new
//!   version restarts the count.
//! - **Unanswered dispute:** M days (14) after the day a dispute was opened,
//!   while it is neither answered nor agreed and no legal case flag of type
//!   *deposit dispute* is open, the property manager's manager is told: the
//!   managers set on the staff profile of the property managers of the
//!   building, else the company admin. Once per dispute.
//!
//! The rule only tells people: it never refunds or forfeits the deposit.
//! There is no separate reminder at 7 days: the escalation at M covers the
//! response window of the property manager.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role;
use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::deposits::{
    DepositDeductionsDisputed, DepositDeductionsPublished, DepositDisputeAnswered, DepositSettled,
};
use wl_kernel::documents::today;
use wl_kernel::workforce::workforce;

use crate::lease::{LEASE_SUBJECT_TYPE, Lease};
use crate::legal_case::{CaseStatus, CaseType, LegalCase};
use crate::notices;

/// The scheduled task and the command.
pub const TASK: &str = "property:deposit-disputes";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";

/// The rule.
pub const RULE: &str = "PRP.RUL.11";

/// N: the days after the deductions are published before the accountant is
/// told that the settlement may proceed.
pub const WINDOW_DAYS: i64 = 14;

/// M: the days a dispute may stay without an answer before it is escalated.
pub const ESCALATE_DAYS: i64 = 14;

/// What the vertical knows of the deductions of one deposit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_deposit_watches", unique(deposit_id), index(root_id))]
pub struct DepositWatch {
    /// The primary key.
    pub id: i64,
    /// The id of version 1: the lease the deposit is held for.
    pub root_id: i64,
    /// The deposit.
    pub deposit_id: i64,
    /// Whose deposit it is.
    pub party_id: i64,
    /// The latest published version.
    pub version: i32,
    /// The day it was published.
    pub published_on: NaiveDate,
    /// The last day the tenant may dispute it.
    pub window_ends_on: Option<NaiveDate>,
    /// The day the accountant was told the settlement may proceed.
    pub notified_on: Option<NaiveDate>,
    /// The day the open dispute was opened; empty when none is open.
    pub dispute_opened_on: Option<NaiveDate>,
    /// The day the open dispute was escalated.
    pub escalated_on: Option<NaiveDate>,
    /// The day the deposit was settled.
    pub closed_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DepositWatch {
    /// The watch of the deposit `deposit_id`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_deposit(db: &Db, deposit_id: i64) -> Result<Option<DepositWatch>> {
        DepositWatch::query()
            .where_(DepositWatch::DEPOSIT_ID.eq(deposit_id))
            .first(db)
            .await
    }

    /// Whether the accountant is due to be told on `day`: the deposit is not
    /// settled, no line is disputed, nobody was told for this version, and
    /// [`WINDOW_DAYS`] passed since it was published.
    pub fn accountant_due(&self, day: NaiveDate) -> bool {
        self.closed_on.is_none()
            && self.dispute_opened_on.is_none()
            && self.notified_on.is_none()
            && self.published_on + Duration::days(WINDOW_DAYS) <= day
    }

    /// Whether the open dispute is due to be escalated on `day`:
    /// [`ESCALATE_DAYS`] passed since it was opened and nobody was told.
    pub fn escalation_due(&self, day: NaiveDate) -> bool {
        self.closed_on.is_none()
            && self.escalated_on.is_none()
            && self
                .dispute_opened_on
                .is_some_and(|opened| opened + Duration::days(ESCALATE_DAYS) <= day)
    }
}

/// The lease a deposit event is about, when it is held for a lease.
async fn lease_of(db: &Db, source: &wl_kernel::Subject) -> Result<Option<Lease>> {
    if source.subject_type != LEASE_SUBJECT_TYPE {
        return Ok(None);
    }
    Lease::find(db, source.subject_id).await
}

/// New deductions were published: the latest version and its day are stored,
/// and the count of N days starts again.
///
/// # Errors
///
/// The database's errors.
pub async fn on_published(state: &AppState, event: &DepositDeductionsPublished) -> Result {
    let Some(lease) = lease_of(&state.db, &event.source).await? else {
        return Ok(());
    };
    match DepositWatch::of_deposit(&state.db, event.deposit.0).await? {
        Some(watch) => {
            DepositWatch::query()
                .where_(DepositWatch::ID.eq(watch.id))
                .update(
                    &state.db,
                    &[
                        ("version", &event.version),
                        ("published_on", &event.published_on),
                        ("window_ends_on", &event.window_ends_on),
                        ("notified_on", &None::<NaiveDate>),
                        ("closed_on", &None::<NaiveDate>),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        None => {
            DepositWatch::create(
                &state.db,
                DepositWatch {
                    root_id: lease.root_id,
                    deposit_id: event.deposit.0,
                    party_id: event.party.0,
                    version: event.version,
                    published_on: event.published_on,
                    window_ends_on: event.window_ends_on,
                    ..Default::default()
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// The tenant disputed lines: the day is stored, and the count of M days
/// starts.
///
/// # Errors
///
/// The database's errors.
pub async fn on_disputed(state: &AppState, event: &DepositDeductionsDisputed) -> Result {
    if lease_of(&state.db, &event.source).await?.is_none() {
        return Ok(());
    }
    DepositWatch::query()
        .where_(DepositWatch::DEPOSIT_ID.eq(event.deposit.0))
        .update(
            &state.db,
            &[
                ("dispute_opened_on", &Some(event.raised_on)),
                ("escalated_on", &None::<NaiveDate>),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Staff answered the dispute: when no line is disputed any more, the dispute
/// is over.
///
/// # Errors
///
/// The database's errors.
pub async fn on_answered(state: &AppState, event: &DepositDisputeAnswered) -> Result {
    if event.still_open || lease_of(&state.db, &event.source).await?.is_none() {
        return Ok(());
    }
    DepositWatch::query()
        .where_(DepositWatch::DEPOSIT_ID.eq(event.deposit.0))
        .update(
            &state.db,
            &[
                ("dispute_opened_on", &None::<NaiveDate>),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The deposit was settled: the rule has nothing more to say about it.
///
/// # Errors
///
/// The database's errors.
pub async fn on_settled(state: &AppState, event: &DepositSettled) -> Result {
    if lease_of(&state.db, &event.source).await?.is_none() {
        return Ok(());
    }
    DepositWatch::query()
        .where_(DepositWatch::DEPOSIT_ID.eq(event.deposit.0))
        .update(
            &state.db,
            &[
                ("closed_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// The users to tell about an unanswered dispute on `lease`: the managers set
/// on the staff profile of the property managers of its building; when there
/// are none, the company admin.
///
/// # Errors
///
/// The database's and the workforce's errors.
pub async fn escalation_recipients(state: &AppState, lease: &Lease) -> Result<Vec<i64>> {
    let staff = workforce(state);
    let property_managers = notices::managers(state, lease).await?;
    let mut found: BTreeSet<i64> = BTreeSet::new();
    for user in &property_managers {
        let Some(employee) = staff.employee_of_user(&state.db, *user).await? else {
            continue;
        };
        if let Some(profile) = staff.staff(state, employee).await? {
            found.extend(profile.managers.iter().copied().filter(|m| m != user));
        }
    }
    if found.is_empty() {
        found.extend(
            users_with_role(&state.db, COMPANY_ADMIN_ROLE)
                .await?
                .iter()
                .map(|admin| admin.id),
        );
    }
    Ok(found.into_iter().collect())
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Accountants told that a settlement may proceed.
    pub ready: usize,
    /// Disputes escalated.
    pub escalated: usize,
    /// Deposits that failed; the run moves on.
    pub failed: usize,
}

/// One deposit on `day`.
async fn check(state: &AppState, watch: &DepositWatch, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let Some(lease) = Lease::find(&state.db, watch.root_id).await? else {
        return Ok(report);
    };
    if watch.accountant_due(day) {
        // Claimed first: of two runs, one tells.
        let claimed = DepositWatch::query()
            .where_(DepositWatch::ID.eq(watch.id))
            .where_(DepositWatch::NOTIFIED_ON.is_null())
            .where_(DepositWatch::VERSION.eq(watch.version))
            .update(
                &state.db,
                &[
                    ("notified_on", &Some(day)),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        if claimed > 0 {
            notices::deposit_may_proceed(state, &lease, watch.version, watch.published_on).await?;
            report.ready += 1;
        }
    }
    if watch.escalation_due(day) {
        let case_open = LegalCase::query()
            .where_(LegalCase::ROOT_ID.eq(watch.root_id))
            .where_(LegalCase::CASE_TYPE.eq(CaseType::DepositDispute))
            .where_(LegalCase::STATUS.eq(CaseStatus::Open))
            .exists(&state.db)
            .await?;
        if !case_open
            && let Some(opened) = watch.dispute_opened_on
        {
            let claimed = DepositWatch::query()
                .where_(DepositWatch::ID.eq(watch.id))
                .where_(DepositWatch::ESCALATED_ON.is_null())
                .where_(DepositWatch::DISPUTE_OPENED_ON.eq(opened))
                .update(
                    &state.db,
                    &[
                        ("escalated_on", &Some(day)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
            if claimed > 0 {
                let users = escalation_recipients(state, &lease).await?;
                notices::dispute_unanswered(state, &lease, &users, opened, ESCALATE_DAYS).await?;
                report.escalated += 1;
            }
        }
    }
    Ok(report)
}

/// `PRP.RUL.11` for `day`. Every deposit is tried; a failure is counted and
/// the run moves on.
///
/// # Errors
///
/// The database's errors.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut total = Report::default();
    let watches = DepositWatch::query()
        .where_(DepositWatch::CLOSED_ON.is_null())
        .order_by(DepositWatch::ID)
        .get(&state.db)
        .await?;
    for watch in watches {
        match check(state, &watch, day).await {
            Ok(report) => {
                total.ready += report.ready;
                total.escalated += report.escalated;
            }
            Err(error) => {
                total.failed += 1;
                eprintln!("{TASK}: deposit {}: {error:?}", watch.deposit_id);
            }
        }
    }
    Ok(total)
}

/// Registers the daily task, its command and the listeners of the deposit
/// events.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Tell the accountant when a deposit settlement may proceed, and escalate unanswered deductions disputes (--date YYYY-MM-DD checks another day)",
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
                "{} settlements may proceed, {} disputes escalated, {} failed.",
                report.ready, report.escalated, report.failed
            );
            Ok(())
        },
    );
    app.listen(|event: DepositDeductionsPublished, state: AppState| async move {
        on_published(&state, &event).await
    });
    app.listen(|event: DepositDeductionsDisputed, state: AppState| async move {
        on_disputed(&state, &event).await
    });
    app.listen(|event: DepositDisputeAnswered, state: AppState| async move {
        on_answered(&state, &event).await
    });
    app.listen(|event: DepositSettled, state: AppState| async move {
        on_settled(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn watch() -> DepositWatch {
        DepositWatch {
            published_on: day(3, 1),
            ..Default::default()
        }
    }

    #[test]
    fn the_accountant_is_told_fourteen_days_after_the_publication() {
        assert!(!watch().accountant_due(day(3, 14)));
        assert!(watch().accountant_due(day(3, 15)));
        let told = DepositWatch {
            notified_on: Some(day(3, 15)),
            ..watch()
        };
        assert!(!told.accountant_due(day(3, 20)));
        let disputed = DepositWatch {
            dispute_opened_on: Some(day(3, 5)),
            ..watch()
        };
        assert!(!disputed.accountant_due(day(3, 20)));
        let settled = DepositWatch {
            closed_on: Some(day(3, 10)),
            ..watch()
        };
        assert!(!settled.accountant_due(day(3, 20)));
    }

    #[test]
    fn a_dispute_is_escalated_fourteen_days_after_it_was_opened() {
        let disputed = DepositWatch {
            dispute_opened_on: Some(day(3, 5)),
            ..watch()
        };
        assert!(!disputed.escalation_due(day(3, 18)));
        assert!(disputed.escalation_due(day(3, 19)));
        let told = DepositWatch {
            escalated_on: Some(day(3, 19)),
            ..disputed.clone()
        };
        assert!(!told.escalation_due(day(3, 25)));
        assert!(!watch().escalation_due(day(4, 1)));
    }
}
