//! The free time of a container and the demurrage it costs (03 Logistics §4.2
//! row *Container free time*, §7.2 `LOG.RUL.14`, §9 *VAT* and
//! *Disbursements*; BL-LOG-24).
//!
//! - **The rule** ([`run_free_time_alerts`], daily at 06:00 Asia/Dubai): for a
//!   container of a job in any state but `closed` and `cancelled`, each of its
//!   three free-time end dates (port storage, demurrage, detention) that has
//!   not been stopped by the container being collected (storage, demurrage) or
//!   returned empty (detention): [`NOTICE_DAYS`] days before the date and on
//!   it, the documentation officers and the client are told, once for each
//!   container, charge type, end date and moment ([`FreeTimeNotice`]); after
//!   the date no notice goes out and one [`FreeTimeLine`] is added for each
//!   day, so the later supplier bill can be checked against it.
//! - **Changed end dates** ([`set_end_dates`]): an extension is audited as an
//!   event of the job; the notices are worked out from the dates as they are
//!   now, so a new date has its own two notices, the old date's unsent notice
//!   is never sent, the log lines already written stay (with a note) and no
//!   line is written for the days before the new date.
//! - **The log stops** when the container is recorded as collected
//!   ([`record_collected`]) or returned empty ([`record_returned_empty`]) on a
//!   date: the line of that date stays, or is written by the run when it has
//!   not passed, and none is written for later dates.
//! - **The bill** ([`add_bill_cost`]): a charge line of a supplier bill (the
//!   kernel's `SupplierBills`, `FIN.PAB.01`) is allocated to the job and the
//!   container as a cost, and the liability with its reason is required to
//!   approve it for the job. A client liability can be recharged (the invoice
//!   of [`crate::freight_reopen::recharge_invoice`], or the supplementary
//!   invoice of a reopened job), as a disbursement `out_of_scope` only with the
//!   conditions of VATP013 recorded; a company liability is never recharged
//!   and lowers the margin. [`logged_days`] gives the days of the log to show
//!   next to the days the bill charges; a difference is shown, not blocked.

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{self, SupplierBillLineId, SupplierBillStatus};

use crate::freight::{FreightContainer, FreightJob, FreightStatus, track};
use crate::freight_costing::{self, JobLine, closed_before, line_kind, liability, liability_reason};
use crate::freight_flow::{load, tell_client};
use crate::freight_hold::tell_officers;
use crate::permissions;
use crate::texts::english;

/// The charge types of a container.
pub mod charge {
    /// Port storage: charged by the port after the free days.
    pub const PORT_STORAGE: &str = "port_storage";
    /// Demurrage: charged by the shipping line for a full container kept in
    /// the port.
    pub const DEMURRAGE: &str = "demurrage";
    /// Detention: charged by the shipping line for a container kept outside
    /// the port.
    pub const DETENTION: &str = "detention";
    /// Every charge type.
    pub const ALL: [&str; 3] = [PORT_STORAGE, DEMURRAGE, DETENTION];
}

/// The name of the scheduled task of `LOG.RUL.14`.
pub const FREE_TIME_TASK: &str = "logistics-freight-free-time";

/// When the rule runs, Asia/Dubai.
pub const FREE_TIME_TASK_AT: &str = "06:00";

/// `N` of `LOG.RUL.14`: the days before an end date the first notice goes out.
pub const NOTICE_DAYS: i64 = 2;

/// One day past an end date: the free-time log of a job.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_free_time_log", index(job_id))]
pub struct FreeTimeLine {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// The container.
    #[model(references = "logistics_freight_containers")]
    pub container_id: i64,
    /// One of [`charge`].
    pub charge_type: String,
    /// The day.
    pub log_date: NaiveDate,
    /// A note: the end date was extended after the line was written.
    pub note: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A notice that went out for an end date.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_free_time_notices", index(job_id))]
pub struct FreeTimeNotice {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// The container.
    #[model(references = "logistics_freight_containers")]
    pub container_id: i64,
    /// One of [`charge`].
    pub charge_type: String,
    /// The end date the notice is about.
    pub end_date: NaiveDate,
    /// `before` or `on`.
    pub moment: String,
    /// The day it was sent.
    pub sent_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a run of the rule did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FreeTimeRun {
    /// Notices sent (each goes to the officers and the client).
    pub notices: usize,
    /// Log lines added.
    pub lines: usize,
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// The states in which a job is open for `LOG.RUL.14`.
const OPEN: [FreightStatus; 7] = [
    FreightStatus::Booked,
    FreightStatus::DocumentsPending,
    FreightStatus::InTransit,
    FreightStatus::CustomsClearance,
    FreightStatus::ReadyForDelivery,
    FreightStatus::OnHold,
    FreightStatus::Delivered,
];

/// The end date and the day the charge stopped, for each charge type of the
/// container.
fn schedule(container: &FreightContainer) -> [(&'static str, Option<NaiveDate>, Option<NaiveDate>); 3] {
    [
        (charge::PORT_STORAGE, container.port_storage_end, container.collected_on),
        (charge::DEMURRAGE, container.demurrage_end, container.collected_on),
        (charge::DETENTION, container.detention_end, container.returned_empty_on),
    ]
}

/// The log of the job, in the order the days were written.
///
/// # Errors
///
/// Database errors.
pub async fn log_of(db: &Db, job_id: i64) -> Result<Vec<FreeTimeLine>> {
    FreeTimeLine::query()
        .where_(FreeTimeLine::JOB_ID.eq(job_id))
        .order_by(FreeTimeLine::ID)
        .get(db)
        .await
}

/// The days of the log for the container and charge type: what to show next
/// to the days a supplier bill charges.
///
/// # Errors
///
/// Database errors.
pub async fn logged_days(db: &Db, container_id: i64, charge_type: &str) -> Result<i64> {
    let count = FreeTimeLine::query()
        .where_(FreeTimeLine::CONTAINER_ID.eq(container_id))
        .where_(FreeTimeLine::CHARGE_TYPE.eq(charge_type))
        .count(db)
        .await?;
    Ok(i64::try_from(count).unwrap_or(i64::MAX))
}

/// The word of a charge type for a text.
fn charge_word(charge_type: &str) -> String {
    charge_type.replace('_', " ")
}

/// `LOG.RUL.14` on `day` (an Asia/Dubai date) with [`NOTICE_DAYS`]; see the
/// module text.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn run_free_time_alerts(state: &AppState, day: NaiveDate) -> Result<FreeTimeRun> {
    run_with(state, day, NOTICE_DAYS).await
}

/// [`run_free_time_alerts`] with `before` days instead of [`NOTICE_DAYS`].
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn run_with(state: &AppState, day: NaiveDate, before: i64) -> Result<FreeTimeRun> {
    let mut done = FreeTimeRun::default();
    for status in OPEN {
        let jobs = FreightJob::query()
            .where_(FreightJob::STATUS.eq(status))
            .order_by(FreightJob::ID)
            .get(&state.db)
            .await?;
        for job in jobs {
            for container in job.containers(&state.db).await? {
                for (charge_type, end, stop) in schedule(&container) {
                    let Some(end) = end else { continue };
                    let running = match stop {
                        Some(stop) => day <= stop,
                        None => true,
                    };
                    if !running {
                        continue;
                    }
                    if day > end {
                        if add_line(state, &job, &container, charge_type, day).await? {
                            done.lines += 1;
                        }
                        continue;
                    }
                    let moment = if day == end {
                        "on"
                    } else if day == end - TimeDelta::days(before) {
                        "before"
                    } else {
                        continue;
                    };
                    if notify_once(state, &job, &container, charge_type, end, moment, day, before)
                        .await?
                    {
                        done.notices += 1;
                    }
                }
            }
        }
    }
    Ok(done)
}

/// Writes the log line of `day`; false when it is already there.
async fn add_line(
    state: &AppState,
    job: &FreightJob,
    container: &FreightContainer,
    charge_type: &str,
    day: NaiveDate,
) -> Result<bool> {
    let known = FreeTimeLine::query()
        .where_(FreeTimeLine::CONTAINER_ID.eq(container.id))
        .where_(FreeTimeLine::CHARGE_TYPE.eq(charge_type))
        .where_(FreeTimeLine::LOG_DATE.eq(day))
        .first(&state.db)
        .await?;
    if known.is_some() {
        return Ok(false);
    }
    let mut line = FreeTimeLine {
        job_id: job.id,
        container_id: container.id,
        charge_type: charge_type.to_owned(),
        log_date: day,
        ..FreeTimeLine::default()
    };
    line.insert(&state.db).await?;
    Ok(true)
}

/// Tells the officers and the client once for the container, charge type, end
/// date and moment; false when it was told before.
#[allow(clippy::too_many_arguments)]
async fn notify_once(
    state: &AppState,
    job: &FreightJob,
    container: &FreightContainer,
    charge_type: &str,
    end: NaiveDate,
    moment: &str,
    day: NaiveDate,
    before: i64,
) -> Result<bool> {
    let sent = FreeTimeNotice::query()
        .where_(FreeTimeNotice::CONTAINER_ID.eq(container.id))
        .where_(FreeTimeNotice::CHARGE_TYPE.eq(charge_type))
        .where_(FreeTimeNotice::END_DATE.eq(end))
        .where_(FreeTimeNotice::MOMENT.eq(moment))
        .first(&state.db)
        .await?;
    if sent.is_some() {
        return Ok(false);
    }
    let word = charge_word(charge_type);
    let when = if moment == "on" {
        english("logistics.freight.free_time_today", &[])
    } else {
        english("logistics.freight.free_time_in_days", &[("days", &before)])
    };
    let title = english(
        "logistics.freight.free_time_title",
        &[
            ("number", &job.number),
            ("container", &container.number),
            ("charge", &word),
        ],
    );
    let body = english(
        "logistics.freight.free_time_body",
        &[
            ("number", &job.number),
            ("container", &container.number),
            ("charge", &word),
            ("end", &end),
            ("when", &when),
        ],
    );
    tell_officers(state, job, &title, &body).await?;
    tell_client(state, job, &title, &body).await?;
    let mut row = FreeTimeNotice {
        job_id: job.id,
        container_id: container.id,
        charge_type: charge_type.to_owned(),
        end_date: end,
        moment: moment.to_owned(),
        sent_on: day,
        ..FreeTimeNotice::default()
    };
    row.insert(&state.db).await?;
    Ok(true)
}

/// Registers the daily rule on the scheduler.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(FREE_TIME_TASK_AT, FREE_TIME_TASK, |state: AppState| async move {
            run_free_time_alerts(&state, today()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
}

/// The container and its job, which must be open (not closed, not cancelled).
async fn open_container(
    state: &AppState,
    container_id: i64,
) -> std::result::Result<(FreightContainer, FreightJob), TransitionError> {
    let container = FreightContainer::find(&state.db, container_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let job = load(state, container.job_id).await?;
    if matches!(job.status, FreightStatus::Closed | FreightStatus::Cancelled) {
        return Err(guard(english("logistics.freight.free_time_closed", &[])));
    }
    Ok((container, job))
}

/// `user` (`jobs.manage`) changes the free-time end dates of the container
/// `container_id`: port storage, demurrage and detention, each left as it is
/// when `None`. Each change is an event of the job; the notices follow the new
/// date and the log lines already written get a note.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a job that is closed
/// or cancelled.
pub async fn set_end_dates(
    state: &AppState,
    user: &AuthUser,
    container_id: i64,
    dates: [Option<NaiveDate>; 3],
) -> std::result::Result<FreightContainer, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let (container, job) = open_container(state, container_id).await?;
    let columns = [
        (charge::PORT_STORAGE, "port_storage_end"),
        (charge::DEMURRAGE, "demurrage_end"),
        (charge::DETENTION, "detention_end"),
    ];
    for (((charge_type, column), new), (_, old, _)) in
        columns.iter().zip(dates).zip(schedule(&container))
    {
        let Some(new) = new else { continue };
        if old == Some(new) {
            continue;
        }
        let value = Some(new);
        FreightContainer::query()
            .where_(FreightContainer::ID.eq(container_id))
            .update(&state.db, &[(*column, &value)])
            .await?;
        let was = old.map_or_else(|| "-".to_owned(), |old| old.to_string());
        let note = english(
            "logistics.freight.free_time_extended",
            &[("charge", &charge_word(charge_type)), ("end", &new)],
        );
        FreeTimeLine::query()
            .where_(FreeTimeLine::CONTAINER_ID.eq(container_id))
            .where_(FreeTimeLine::CHARGE_TYPE.eq(*charge_type))
            .update(&state.db, &[("note", &note)])
            .await?;
        track(
            &state.db,
            job.id,
            "free_time_changed",
            &format!("{} {charge_type}: {was} -> {new}", container.number),
            Some(user.id),
        )
        .await?;
    }
    FreightContainer::find(&state.db, container_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`jobs.manage`) records that the container was collected on `on`:
/// the log of port storage and demurrage stops after that day.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a job that is closed
/// or cancelled.
pub async fn record_collected(
    state: &AppState,
    user: &AuthUser,
    container_id: i64,
    on: NaiveDate,
) -> std::result::Result<FreightContainer, TransitionError> {
    record(state, user, container_id, on, "collected_on", "collected").await
}

/// `user` (`jobs.manage`) records that the container was returned empty on
/// `on`: the log of detention stops after that day.
///
/// # Errors
///
/// As [`record_collected`].
pub async fn record_returned_empty(
    state: &AppState,
    user: &AuthUser,
    container_id: i64,
    on: NaiveDate,
) -> std::result::Result<FreightContainer, TransitionError> {
    record(state, user, container_id, on, "returned_empty_on", "returned_empty").await
}

async fn record(
    state: &AppState,
    user: &AuthUser,
    container_id: i64,
    on: NaiveDate,
    column: &str,
    kind: &str,
) -> std::result::Result<FreightContainer, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let (container, job) = open_container(state, container_id).await?;
    let value = Some(on);
    FreightContainer::query()
        .where_(FreightContainer::ID.eq(container_id))
        .update(&state.db, &[(column, &value)])
        .await?;
    track(
        &state.db,
        job.id,
        kind,
        &format!("{}: {on}", container.number),
        Some(user.id),
    )
    .await?;
    FreightContainer::find(&state.db, container_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// What [`add_bill_cost`] takes: the charge line of the supplier bill and how
/// it is allocated.
#[derive(Debug, Clone, Default)]
pub struct BillCost {
    /// The supplier bill line (`FIN.PAB.01`).
    pub bill_line_id: i64,
    /// The number of the container of the job it is for.
    pub container: String,
    /// One of [`charge`].
    pub charge_type: String,
    /// The days the bill charges.
    pub billed_days: i64,
    /// `client` or `company`.
    pub liability: String,
    /// One of [`liability_reason`].
    pub reason: String,
    /// Whether a client liability is recharged.
    pub rechargeable: bool,
}

/// `user` (`jobs.manage`) allocates a charge line of a supplier bill to the
/// job `job_id` and one of its containers as a cost. The liability and its
/// reason are required before the bill is approved for the job, so the cost
/// is allocated at once; a company liability is never rechargeable.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a closed job (it is
/// reopened first), an unknown container, charge type, bill line or voided
/// bill, a bill line already allocated, a missing liability or a reason that
/// does not fit it.
pub async fn add_bill_cost(
    state: &AppState,
    user: &AuthUser,
    job_id: i64,
    input: &BillCost,
) -> std::result::Result<JobLine, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let job = load(state, job_id).await?;
    if job.status == FreightStatus::Closed {
        return Err(guard(english("logistics.freight.closed_no_costs", &[])));
    }
    let charge_type = input.charge_type.trim().to_owned();
    if !charge::ALL.contains(&charge_type.as_str()) || input.billed_days < 0 {
        return Err(guard(english("logistics.freight.charge_type_unknown", &[])));
    }
    let number = input.container.trim().to_ascii_uppercase();
    let container = FreightContainer::query()
        .where_(FreightContainer::JOB_ID.eq(job_id))
        .where_(FreightContainer::NUMBER.eq(number.as_str()))
        .first(&state.db)
        .await?
        .ok_or_else(|| guard(english("logistics.freight.container_unknown", &[])))?;
    let liability_of = input.liability.trim();
    let reason = input.reason.trim();
    if !liability::ALL.contains(&liability_of) {
        return Err(guard(english("logistics.freight.bill_liability_needed", &[])));
    }
    if !liability_reason::fits(liability_of, reason) {
        return Err(guard(english("logistics.freight.reason_unfit", &[])));
    }
    let bill = invoicing::supplier_bills(state)
        .line(&state.db, SupplierBillLineId(input.bill_line_id))
        .await?
        .filter(|bill| bill.bill_status != SupplierBillStatus::Void)
        .ok_or_else(|| guard(english("logistics.freight.bill_line_unknown", &[])))?;
    let source_ref = format!("{} / line {}", bill.bill_number, input.bill_line_id);
    let used = JobLine::query()
        .where_(JobLine::SOURCE_REF.eq(source_ref.as_str()))
        .first(&state.db)
        .await?;
    if used.is_some() {
        return Err(guard(english("logistics.freight.bill_line_used", &[])));
    }
    let mut line = JobLine {
        job_id,
        kind: line_kind::COST.to_owned(),
        description: bill.description.clone(),
        amount: bill.amount_aed,
        liability: Some(liability_of.to_owned()),
        allocated: true,
        rechargeable: input.rechargeable && liability_of == liability::CLIENT,
        source_ref: Some(source_ref),
        late: closed_before(&state.db, job_id).await?,
        created_by: Some(user.id),
        container_id: Some(container.id),
        charge_type: Some(charge_type),
        billed_days: Some(input.billed_days),
        liability_reason: Some(reason.to_owned()),
        ..JobLine::default()
    };
    line.insert(&state.db).await?;
    if job.status == FreightStatus::Cancelled {
        crate::freight_cancel::tell_accountants(state, &job, &line).await?;
    }
    Ok(line)
}

/// How many days the log has for a container charge line next to the days its
/// bill charges: `(logged, billed)`; `None` for a line that is no container
/// charge.
///
/// # Errors
///
/// Database errors.
pub async fn days_compared(db: &Db, line: &JobLine) -> Result<Option<(i64, i64)>> {
    let (Some(container), Some(charge_type), Some(billed)) =
        (line.container_id, line.charge_type.as_deref(), line.billed_days)
    else {
        return Ok(None);
    };
    Ok(Some((logged_days(db, container, charge_type).await?, billed)))
}
