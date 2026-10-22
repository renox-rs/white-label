//! The credits a student's attendance uses (10 §4.2 row *A student is absent*,
//! §7.1 *Session* `submit_attendance` and `correct_attendance`, `SAL.PKG.01`;
//! BL-EDU-15, BL-EDU-14).
//!
//! The mark of a student says what it costs now (`credit_due`, by the absence
//! policy stored with the enrolment, [`crate::attendance::credit_due`]); the
//! ledger of wl-sales holds what has been taken (`consumed`). [`settle`] moves the
//! ledger to match, through the kernel's `PackageLedger`, **after the commit** of
//! the run that stored the mark (money steps come after it):
//!
//! - a credit is due and none is taken: `consume` one for the source
//!   `edtech.attendance:<row>`; a source that consumed already changes nothing;
//! - none is due and one is taken: `reverse` it (a correction), and a package
//!   that was `completed` at zero comes back to `active` in wl-sales;
//! - the package has no credit left (or is unpaid and consumption is suspended):
//!   the mark and the correction stay as they are, the row says why
//!   (`credit_note`) and the counsellor gets **one task** to sell credits; the
//!   job `edtech:credits` tries again, so a package sold later is consumed and the
//!   task closes.
//!
//! Nothing is consumed for a cancelled session (it has no marks), for a
//! membership or a course fee (no credits), or for an excused absence.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::sales::{ConsumePackage, PackageConsumption, package_ledger};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::role_tasks;

use crate::attendance::Attendance;
use crate::enrolment::{Enrolment, EnrolmentKind};
use crate::enrolments::party_name;
use crate::permissions;
use crate::texts::english;

/// The scheduled job and command that settle the marks whose credit is not in
/// step with the ledger.
pub const JOB: &str = "edtech:credits";

/// How often [`JOB`] runs.
pub const JOB_EVERY_MINUTES: u64 = 15;

/// The note of a row whose credit could not be taken: the package has none left.
pub const NOTE_NO_CREDIT: &str = "no_credit";

/// The note of a row whose credit could not be taken: the package has a reopened,
/// unpaid balance.
pub const NOTE_UNPAID: &str = "unpaid";

/// The key the one open task to sell credits to `enrolment` is found by.
pub fn task_key(enrolment: i64) -> String {
    format!("edtech.credits:{enrolment}")
}

/// Sets the ledger side of a row.
async fn store(
    state: &AppState,
    id: i64,
    consumed: bool,
    note: Option<&str>,
) -> Result {
    Attendance::query()
        .where_(Attendance::ID.eq(id))
        .update(
            &state.db,
            &[
                ("consumed", &consumed),
                ("credit_note", &note.map(str::to_owned)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// The counsellor's task to sell credits: one open at a time per enrolment, to the
/// enrolment's counsellor, else the first holder of the counsellor role (the
/// company admin when nobody holds it).
async fn sell_credits_task(state: &AppState, enrolment: &Enrolment, note: &str) -> Result {
    let creator = task_creator(state);
    let key = task_key(enrolment.id);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let student = party_name(state, "en", enrolment.student_party_id).await;
    let item = enrolment
        .item_name
        .clone()
        .or_else(|| enrolment.item_code.clone())
        .unwrap_or_default();
    let reason = english(&format!("edtech.credits.note.{note}"), &[]);
    let title = english(
        "edtech.credits.task_title",
        &[("student", &student), ("item", &item)],
    );
    let body = english(
        "edtech.credits.task_body",
        &[
            ("student", &student),
            ("item", &item),
            ("enrolment", &enrolment.number()),
            ("reason", &reason),
        ],
    );
    let mut ticket = OpenTicket::new(enrolment.subject(), TASK_CATEGORY, &title)
        .description(&body)
        .source_key(&key);
    let holder = match enrolment.counsellor_id {
        Some(user) => Some(user),
        None => role_tasks::assignee(&state.db, permissions::COUNSELLOR_ROLE).await?,
    };
    if let Some(user) = holder {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// Closes the task to sell credits of `enrolment` once no row of it waits for a
/// credit.
async fn close_task_if_settled(state: &AppState, enrolment: &Enrolment) -> Result {
    let waiting = Attendance::query()
        .where_(Attendance::ENROLMENT_ID.eq(enrolment.id))
        .where_(Attendance::CREDIT_DUE.eq(true))
        .where_(Attendance::CONSUMED.eq(false))
        .exists(&state.db)
        .await?;
    if waiting {
        return Ok(());
    }
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &task_key(enrolment.id)).await? {
        creator
            .complete(state, found.id, &english("edtech.credits.task_done", &[]))
            .await?;
    }
    Ok(())
}

/// Moves the ledger to what row `attendance_id` costs now. Answers whether the
/// ledger is in step with it afterwards.
///
/// # Errors
///
/// The database's, the ledger's and the task creator's errors; the row stays out
/// of step and is tried again by [`settle_pending`].
pub async fn settle(state: &AppState, attendance_id: i64) -> Result<bool> {
    let row = Attendance::find_or_404(&state.db, attendance_id).await?;
    let enrolment = Enrolment::find_or_404(&state.db, row.enrolment_id).await?;
    let Some(item) = enrolment.item_code.clone().filter(|_| enrolment.kind == EnrolmentKind::Package)
    else {
        return Ok(true);
    };
    let ledger = package_ledger(state);
    if row.credit_due && !row.consumed {
        let request = ConsumePackage::new(enrolment.student(), &item, 1, row.subject());
        return match ledger.consume(state, request).await? {
            PackageConsumption::Consumed { .. } | PackageConsumption::Already { .. } => {
                store(state, row.id, true, None).await?;
                close_task_if_settled(state, &enrolment).await?;
                Ok(true)
            }
            PackageConsumption::NoCredit => {
                store(state, row.id, false, Some(NOTE_NO_CREDIT)).await?;
                sell_credits_task(state, &enrolment, NOTE_NO_CREDIT).await?;
                Ok(false)
            }
            PackageConsumption::Unpaid { .. } => {
                store(state, row.id, false, Some(NOTE_UNPAID)).await?;
                sell_credits_task(state, &enrolment, NOTE_UNPAID).await?;
                Ok(false)
            }
        };
    }
    if !row.credit_due && row.consumed {
        // Whether or not the ledger had one to give back, none is held now.
        let _gave_back = ledger.reverse(state, row.subject()).await?;
        store(state, row.id, false, None).await?;
        close_task_if_settled(state, &enrolment).await?;
    }
    Ok(true)
}

/// Settles every row of session `session_id`.
///
/// # Errors
///
/// The first error of the rows, after all were tried.
pub async fn settle_session(state: &AppState, session_id: i64) -> Result {
    let mut failure = None;
    for row in Attendance::of_session(&state.db, session_id).await? {
        if let Err(error) = settle(state, row.id).await {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Settles the rows whose credit is not in step with the ledger: a credit due and
/// not taken (a package sold since, a ledger that was down), a credit taken and no
/// longer due. Answers how many were put in step.
///
/// # Errors
///
/// The first error of the rows, after all were tried.
pub async fn settle_pending(state: &AppState) -> Result<usize> {
    let mut rows = Attendance::query()
        .where_(Attendance::CREDIT_DUE.eq(true))
        .where_(Attendance::CONSUMED.eq(false))
        .order_by(Attendance::ID)
        .get(&state.db)
        .await?;
    rows.extend(
        Attendance::query()
            .where_(Attendance::CREDIT_DUE.eq(false))
            .where_(Attendance::CONSUMED.eq(true))
            .order_by(Attendance::ID)
            .get(&state.db)
            .await?,
    );
    let mut done = 0;
    let mut failure = None;
    for row in rows {
        match settle(state, row.id).await {
            Ok(true) => done += 1,
            Ok(false) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    failure.map_or(Ok(done), Err)
}

/// Registers the job and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .every_minutes(JOB_EVERY_MINUTES, JOB, |state: AppState| async move {
            settle_pending(&state).await.map(|_| ())
        });
    app.command(
        JOB,
        "Take or give back the package credits that the attendance marks cost and the ledger does not hold",
        |_args, state: AppState| async move {
            let done = settle_pending(&state).await?;
            println!("{done} marks settled.");
            Ok(())
        },
    );
}
