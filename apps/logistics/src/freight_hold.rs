//! Holding a freight job and releasing it (03 Logistics §4.2 rows *Customs
//! hold, inspection, declaration rejected, duty unpaid, documents missing* and
//! *Mirsal or ATLP down*, §7.1 rows `hold` and `release`, §7.2 `LOG.RUL.09`,
//! §8 *Integration down*; BL-LOG-21).
//!
//! - **Hold** ([`hold`]): `jobs.manage` puts a job that is `in_transit`,
//!   `customs_clearance` or `ready_for_delivery` `on_hold` with a reason
//!   ([`reason`]) and a note. The state before the hold is written on the job
//!   (`held_from`) and as a [`FreightHold`] period in the same transaction as
//!   the state change. The client and the documentation officers are told
//!   after the commit and the portal shows the reason label and the note
//!   ([`reason_label`]). `booked` and `documents_pending` cannot be held
//!   (documents missing before departure is `documents_pending`).
//! - **Another reason** ([`add_reason`]): a job already on hold is not held
//!   again; a second reason is added as a note on the open hold.
//! - **Release** ([`release`]): `jobs.manage`, only with a note on how the
//!   reason was resolved, and always to the stored state: [`release_name`]
//!   picks the one of the three release transitions that matches it.
//! - **The clock**: [`run_eta_alerts`] (`LOG.RUL.09`, daily) tells the
//!   documentation officers when the ETA is reached and the job is not
//!   `ready_for_delivery` and not `on_hold`, once for each ETA; a job released
//!   after its ETA passed is alerted at once against the ETA as recorded.
//!   Hold periods are stored and [`days_on_hold`] leaves them out of
//!   [`is_on_time`].

use renox::auth::permissions::users_with_permission;
use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::freight::{FreightJob, FreightStatus, track};
use crate::freight_flow::{load, run, tell_client};
use crate::freight_machine::{
    FreightInput, HOLD, RELEASE_CUSTOMS_CLEARANCE, RELEASE_IN_TRANSIT, RELEASE_READY_FOR_DELIVERY,
    given, input_of, needs, trimmed,
};
use crate::notices::tell_user;
use crate::permissions;
use crate::texts::{english, t};

/// The scheduled task of `LOG.RUL.09`.
pub const ETA_TASK: &str = "logistics-freight-eta";

/// The time the rule runs, Asia/Dubai.
pub const ETA_TASK_AT: &str = "07:00";

/// Why a job is held (§6 *Freight job*, hold reason).
pub mod reason {
    /// A customs hold or a physical inspection.
    pub const CUSTOMS_HOLD: &str = "customs_hold_or_inspection";
    /// The customs declaration was rejected.
    pub const DECLARATION_REJECTED: &str = "declaration_rejected";
    /// Duty is unpaid.
    pub const DUTY_UNPAID: &str = "duty_unpaid";
    /// Documents are missing.
    pub const DOCUMENTS_MISSING: &str = "documents_missing";
    /// Mirsal or ATLP is down; the declaration is filed by hand.
    pub const PLATFORM_DOWN: &str = "customs_platform_down";
    /// The client asked to stop.
    pub const CLIENT_INSTRUCTION: &str = "client_instruction";
    /// Every reason.
    pub const ALL: [&str; 6] = [
        CUSTOMS_HOLD,
        DECLARATION_REJECTED,
        DUTY_UNPAID,
        DOCUMENTS_MISSING,
        PLATFORM_DOWN,
        CLIENT_INSTRUCTION,
    ];
}

/// One period a job was on hold.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_holds", index(job_id))]
pub struct FreightHold {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// One of [`reason`].
    pub reason: String,
    /// The note the client reads.
    pub note: String,
    /// The state before the hold, where `release` returns to.
    pub from_state: String,
    /// When the hold began.
    pub started_at: DateTime,
    /// When it ended; none while the job is on hold.
    pub ended_at: Option<DateTime>,
    /// How the reason was resolved.
    pub release_note: Option<String>,
    /// Further reasons added during the hold, one per line.
    pub extra_notes: String,
    /// The user who held the job.
    pub held_by: Option<i64>,
    /// The user who released it.
    pub released_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The label of a hold reason the client reads in the portal, in the person's
/// language.
pub fn reason_label(lang: &Lang, code: &str) -> String {
    t(lang, &format!("logistics.freight.hold_reason.{code}"))
}

/// The English label of a hold reason.
pub fn reason_label_en(code: &str) -> String {
    english(&format!("logistics.freight.hold_reason.{code}"), &[])
}

/// The state a job can be held from, as written on the job.
const HOLDABLE: [FreightStatus; 3] = [
    FreightStatus::InTransit,
    FreightStatus::CustomsClearance,
    FreightStatus::ReadyForDelivery,
];

/// The release transition that returns to `stored`, the state before the
/// hold; the first one when nothing is stored (it then refuses, the job
/// is not on hold).
pub fn release_name(stored: Option<&str>) -> &'static str {
    match stored {
        Some("customs_clearance") => RELEASE_CUSTOMS_CLEARANCE,
        Some("ready_for_delivery") => RELEASE_READY_FOR_DELIVERY,
        _ => RELEASE_IN_TRANSIT,
    }
}

/// The `hold` transition.
pub(crate) fn hold_transition() -> Transition<FreightJob> {
    let transition = Transition::new(HOLD)
        .label("Hold")
        .tone("warning")
        .permission(permissions::JOBS_MANAGE)
        .from(HOLDABLE)
        .to(FreightStatus::OnHold);
    needs(
        transition,
        "reason_given",
        "logistics.freight.hold_reason_needed",
        |_job, input| {
            input
                .reason
                .as_deref()
                .is_some_and(|reason| reason::ALL.contains(&reason.trim()))
        },
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let from = job.status.as_str().to_owned();
        let input = input_of(ctx);
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move {
            let why = trimmed(&input.reason).unwrap_or_default();
            let note = trimmed(&input.note).unwrap_or_default();
            let now = renox::db::now();
            // The state before the hold is written with the state change.
            let why_value = Some(why.clone());
            let note_value = Some(note.clone());
            let from_value = Some(from.clone());
            let at = Some(now);
            FreightJob::query()
                .where_(FreightJob::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("hold_reason", &why_value),
                        ("hold_note", &note_value),
                        ("held_from", &from_value),
                        ("held_at", &at),
                    ],
                )
                .await?;
            let mut period = FreightHold {
                job_id: id,
                reason: why.clone(),
                note: note.clone(),
                from_state: from,
                started_at: now,
                held_by: actor,
                ..FreightHold::default()
            };
            period.insert(&mut *tx).await?;
            track(&mut *tx, id, "held", &format!("{why}: {note}"), actor).await
        })
    })
}

/// A `release` transition to `to`: the state before the hold, how the reason
/// was resolved.
pub(crate) fn release_transition(
    name: &'static str,
    to: FreightStatus,
) -> Transition<FreightJob> {
    let transition = Transition::new(name)
        .label("Release")
        .tone("primary")
        .permission(permissions::JOBS_MANAGE)
        .from([FreightStatus::OnHold])
        .to(to)
        .guard("back_to_the_state_before_the_hold", move |job: &FreightJob, _ctx| {
            let stored = job.held_from.as_deref() == Some(to.as_str());
            Box::pin(async move {
                if stored {
                    Ok(())
                } else {
                    Err(english("logistics.freight.release_target", &[]))
                }
            })
        });
    needs(
        transition,
        "hold_reason_resolved",
        "logistics.freight.release_note_needed",
        |_job, input| given(&input.note),
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let note = trimmed(&input_of(ctx).note);
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move {
            let at = Some(renox::db::now());
            FreightHold::query()
                .where_(FreightHold::JOB_ID.eq(id))
                .where_(FreightHold::ENDED_AT.is_null())
                .update(
                    &mut *tx,
                    &[
                        ("ended_at", &at),
                        ("release_note", &note),
                        ("released_by", &actor),
                    ],
                )
                .await?;
            track(&mut *tx, id, "released", note.as_deref().unwrap_or_default(), actor).await
        })
    })
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// The hold periods of the job, the first first.
///
/// # Errors
///
/// Database errors.
pub async fn holds_of(db: &Db, job_id: i64) -> Result<Vec<FreightHold>> {
    FreightHold::query()
        .where_(FreightHold::JOB_ID.eq(job_id))
        .order_by(FreightHold::ID)
        .get(db)
        .await
}

/// Tells the documentation officers (the holders of `jobs.manage`) of `job`.
pub(crate) async fn tell_officers(state: &AppState, job: &FreightJob, title: &str, body: &str) -> Result {
    for user in users_with_permission(&state.db, permissions::JOBS_MANAGE).await? {
        tell_user(state, job.subject(), &job.number, user.id, title, body).await?;
    }
    Ok(())
}

/// `user` (`jobs.manage`) holds the job `id` for `why` (one of [`reason`])
/// with a `note`. The client and the documentation officers are told; the
/// portal shows the reason label and the note.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is
/// `in_transit`, `customs_clearance` or `ready_for_delivery` (a job already on
/// hold gets [`add_reason`]); `Guard` (422) without a known reason.
pub async fn hold(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    why: &str,
    note: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let input = FreightInput {
        reason: Some(why.trim().to_owned()),
        note: Some(note.trim().to_owned()),
        ..FreightInput::default()
    };
    let job = run(state, user, id, HOLD, &input).await?;
    let label = reason_label_en(why.trim());
    let title = english(
        "logistics.freight.hold_title",
        &[("number", &job.number), ("reason", &label)],
    );
    let body = english(
        "logistics.freight.hold_body",
        &[("number", &job.number), ("reason", &label), ("note", &note.trim())],
    );
    tell_client(state, &job, &title, &body).await?;
    tell_officers(state, &job, &title, &body).await?;
    Ok(job)
}

/// `user` (`jobs.manage`) adds a second reason as a note on the open hold of
/// the job `id`.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) unless the job is on hold,
/// for an unknown reason or a blank note.
pub async fn add_reason(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    why: &str,
    note: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let job = load(state, id).await?;
    if job.status != FreightStatus::OnHold {
        return Err(guard(english("logistics.freight.not_on_hold", &[])));
    }
    if !reason::ALL.contains(&why.trim()) || note.trim().is_empty() {
        return Err(guard(english("logistics.freight.hold_note_needed", &[])));
    }
    let open = FreightHold::query()
        .where_(FreightHold::JOB_ID.eq(id))
        .where_(FreightHold::ENDED_AT.is_null())
        .first(&state.db)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let line = format!("{}: {}", reason_label_en(why.trim()), note.trim());
    let extra = if open.extra_notes.is_empty() {
        line.clone()
    } else {
        format!("{}\n{line}", open.extra_notes)
    };
    FreightHold::query()
        .where_(FreightHold::ID.eq(open.id))
        .update(&state.db, &[("extra_notes", &extra)])
        .await?;
    let shown = Some(if job.hold_note.as_deref().unwrap_or_default().is_empty() {
        line.clone()
    } else {
        format!("{}\n{line}", job.hold_note.clone().unwrap_or_default())
    });
    FreightJob::query()
        .where_(FreightJob::ID.eq(id))
        .update(&state.db, &[("hold_note", &shown)])
        .await?;
    track(&state.db, id, "hold_note", &line, Some(user.id)).await?;
    load(state, id).await
}

/// `user` (`jobs.manage`) releases the job `id` with a note on how the reason
/// was resolved. It returns to the state it was in before the hold, never to
/// another; the client is told.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is `on_hold`;
/// `Guard` (422) without a note.
pub async fn release(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let stored = load(state, id).await?.held_from;
    let input = FreightInput {
        note: Some(note.trim().to_owned()),
        ..FreightInput::default()
    };
    let job = run(state, user, id, release_name(stored.as_deref()), &input).await?;
    let title = english("logistics.freight.release_title", &[("number", &job.number)]);
    let body = english(
        "logistics.freight.release_body",
        &[("number", &job.number), ("note", &note.trim())],
    );
    tell_client(state, &job, &title, &body).await?;
    Ok(job)
}

/// The days the job spent on hold up to `until` (a hold still open counts to
/// `until`), whole days rounded down.
///
/// # Errors
///
/// Database errors.
pub async fn days_on_hold(db: &Db, job_id: i64, until: DateTime) -> Result<i64> {
    let mut total = TimeDelta::zero();
    for period in holds_of(db, job_id).await? {
        let end = period.ended_at.unwrap_or(until).min(until);
        if end > period.started_at {
            total += end - period.started_at;
        }
    }
    Ok(total.num_days())
}

/// Whether the delivered job `job` arrived on time: the delivery is no later
/// than the ETA plus the days it spent on hold (hold periods are left out of
/// on-time measures). `None` for a job with no ETA or not delivered.
///
/// # Errors
///
/// Database errors.
pub async fn is_on_time(db: &Db, job: &FreightJob) -> Result<Option<bool>> {
    let Some(eta) = job.eta else {
        return Ok(None);
    };
    let delivered = job
        .events(db)
        .await?
        .into_iter()
        .rev()
        .find(|event| event.kind == "delivered");
    let Some(delivered) = delivered else {
        return Ok(None);
    };
    let held = days_on_hold(db, job.id, delivered.occurred_at).await?;
    let allowed = eta + TimeDelta::days(held);
    Ok(Some(wl_kernel::documents::dubai_date(delivered.occurred_at) <= allowed))
}

/// `LOG.RUL.09` on `day`: tells the documentation officers of every job whose
/// ETA is reached and that is neither `ready_for_delivery` nor `on_hold` (nor
/// delivered, closed or cancelled), once for each ETA. A job on hold is
/// skipped and alerted after its release, against the ETA as recorded.
/// Returns how many jobs were alerted.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn run_eta_alerts(state: &AppState, day: NaiveDate) -> Result<usize> {
    let mut alerted = 0;
    for status in [
        FreightStatus::Booked,
        FreightStatus::DocumentsPending,
        FreightStatus::InTransit,
        FreightStatus::CustomsClearance,
    ] {
        let jobs = FreightJob::query()
            .where_(FreightJob::STATUS.eq(status))
            .order_by(FreightJob::ID)
            .get(&state.db)
            .await?;
        for job in jobs {
            let Some(eta) = job.eta else { continue };
            if eta > day || job.eta_alerted_for == Some(eta) {
                continue;
            }
            let title = english("logistics.freight.eta_title", &[("number", &job.number)]);
            let body = english(
                "logistics.freight.eta_body",
                &[("number", &job.number), ("eta", &eta)],
            );
            tell_officers(state, &job, &title, &body).await?;
            let marked = Some(eta);
            FreightJob::query()
                .where_(FreightJob::ID.eq(job.id))
                .update(&state.db, &[("eta_alerted_for", &marked)])
                .await?;
            alerted += 1;
        }
    }
    Ok(alerted)
}

/// Registers the daily ETA rule on the scheduler.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(ETA_TASK_AT, ETA_TASK, |state: AppState| async move {
            run_eta_alerts(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
}
