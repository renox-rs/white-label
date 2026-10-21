//! The revenue and cost lines of a freight job and its margin (03 Logistics
//! §4.2 steps 5, 7 and 8, §7.1 rows `close` and `reopen`): the stand-in for
//! job costing (`FIN.CST.01`), supplier bills (`FIN.PAB.01`) and
//! disbursements (`FIN.DSB.01`), which the construction phase (F2) was to
//! provide and did not.
//!
//! A [`JobLine`] is a revenue line (what the client is invoiced; *issued*
//! once its invoice is) or a cost line (a supplier bill or a disbursement;
//! *allocated* once it has a liability, `client` or `company`, and says
//! whether it is rechargeable). `close` is guarded by the lines: every cost
//! allocated and every revenue line issued. The margin at each close is kept
//! as a [`FreightMargin`] (`close_no` 1, 2, ...), reports show the latest.
//!
//! Replacing this module with the real job costing is the whole change: the
//! machine reads only [`open_costs`], [`unissued_revenue`] and writes
//! [`FreightMargin`] at `close`.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};

use crate::freight::{FreightJob, FreightStatus};
use crate::permissions;
use crate::texts::english;

/// The kinds of line.
pub mod line_kind {
    /// What the client is invoiced.
    pub const REVENUE: &str = "revenue";
    /// A supplier bill or a disbursement.
    pub const COST: &str = "cost";
}

/// Who carries a cost (`BL-LOG-24`).
pub mod liability {
    /// Late documents, late duty payment, late collection by the client:
    /// recharged per the quotation terms.
    pub const CLIENT: &str = "client";
    /// Company delay or other: absorbed in the job margin.
    pub const COMPANY: &str = "company";
    /// Every liability.
    pub const ALL: [&str; 2] = [CLIENT, COMPANY];
}

/// A revenue or cost line of a job. Money is fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_job_lines", index(job_id))]
pub struct JobLine {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// `revenue` or `cost` ([`line_kind`]).
    pub kind: String,
    /// What it is.
    pub description: String,
    /// The amount, in fils.
    pub amount: i64,
    /// For a cost: who carries it ([`liability`]); none until allocated.
    pub liability: Option<String>,
    /// Whether the cost has its liability.
    pub allocated: bool,
    /// Whether the cost is recharged to the client.
    pub rechargeable: bool,
    /// For a revenue line: whether its invoice is issued.
    pub issued: bool,
    /// The invoice of a revenue line.
    pub invoice_id: Option<i64>,
    /// The supplier bill or disbursement a cost comes from.
    pub source_ref: Option<String>,
    /// Recorded after the job was closed once (a late cost).
    pub late: bool,
    /// The user who recorded it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The margin of a job at one close.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_margins", unique(job_id, close_no))]
pub struct FreightMargin {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// 1 for the first close, 2 for the close after a reopening...
    pub close_no: i64,
    /// Revenue at that close, in fils.
    pub revenue: i64,
    /// Costs at that close, in fils.
    pub cost: i64,
    /// Revenue less costs, in fils.
    pub margin: i64,
    /// The user who closed it.
    pub closed_by: Option<i64>,
    /// When it was closed.
    pub closed_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Revenue, costs and margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Figures {
    /// Revenue, in fils.
    pub revenue: i64,
    /// Costs, in fils.
    pub cost: i64,
    /// Revenue less costs, in fils.
    pub margin: i64,
}

/// The figures of `lines`: every revenue line and every cost line counts,
/// allocated or not (an unallocated cost blocks `close`, it is not free).
pub fn figures(lines: &[JobLine]) -> Figures {
    let sum = |kind: &str| -> i64 {
        lines
            .iter()
            .filter(|line| line.kind == kind)
            .map(|line| line.amount)
            .sum()
    };
    let revenue = sum(line_kind::REVENUE);
    let cost = sum(line_kind::COST);
    Figures {
        revenue,
        cost,
        margin: revenue - cost,
    }
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// The lines of the job `job_id`, oldest first.
///
/// # Errors
///
/// Database errors.
pub async fn lines_of<'c, E: renox::db::Executor<'c>>(
    executor: E,
    job_id: i64,
) -> Result<Vec<JobLine>> {
    JobLine::query()
        .where_(JobLine::JOB_ID.eq(job_id))
        .order_by(JobLine::ID)
        .get(executor)
        .await
}

/// The figures of the job `job_id` now.
///
/// # Errors
///
/// Database errors.
pub async fn figures_of(db: &Db, job_id: i64) -> Result<Figures> {
    Ok(figures(&lines_of(db, job_id).await?))
}

/// The margins at each close of the job `job_id`, the first close first.
///
/// # Errors
///
/// Database errors.
pub async fn margins_of(db: &Db, job_id: i64) -> Result<Vec<FreightMargin>> {
    FreightMargin::query()
        .where_(FreightMargin::JOB_ID.eq(job_id))
        .order_by(FreightMargin::CLOSE_NO)
        .get(db)
        .await
}

/// How many cost lines of the job have no liability yet: `close` waits for
/// them.
///
/// # Errors
///
/// Database errors.
pub async fn open_costs(db: &Db, job_id: i64) -> Result<i64> {
    let lines = lines_of(db, job_id).await?;
    let open = lines
        .iter()
        .filter(|line| line.kind == line_kind::COST && !line.allocated)
        .count();
    Ok(i64::try_from(open).unwrap_or(i64::MAX))
}

/// How many revenue lines of the job are not issued, and whether the job has
/// any revenue line at all: `close` needs an invoice issued.
///
/// # Errors
///
/// Database errors.
pub async fn invoicing_state(db: &Db, job_id: i64) -> Result<(i64, bool)> {
    let lines = lines_of(db, job_id).await?;
    let revenue: Vec<&JobLine> = lines
        .iter()
        .filter(|line| line.kind == line_kind::REVENUE)
        .collect();
    let unissued = revenue.iter().filter(|line| !line.issued).count();
    Ok((
        i64::try_from(unissued).unwrap_or(i64::MAX),
        !revenue.is_empty(),
    ))
}

async fn job_of(
    db: &Db,
    id: i64,
) -> std::result::Result<FreightJob, TransitionError> {
    FreightJob::find(db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// Whether the job has been closed before: a line recorded now is a late one.
async fn closed_before(db: &Db, job_id: i64) -> Result<bool> {
    Ok(!margins_of(db, job_id).await?.is_empty())
}

/// `user` (`jobs.manage`) records a cost on the job `job_id` (a supplier bill
/// or a disbursement), not yet allocated. A closed job takes none: it is
/// reopened first (`reopen`, BL-LOG-22); a cancelled job takes it (the
/// accountant decides whether to invoice it).
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for an amount of 0 or
/// less, a blank description or a closed job.
pub async fn add_cost(
    state: &AppState,
    user: &AuthUser,
    job_id: i64,
    description: &str,
    amount: i64,
    source_ref: Option<&str>,
) -> std::result::Result<JobLine, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let job = job_of(&state.db, job_id).await?;
    if job.status == FreightStatus::Closed {
        return Err(guard(english("logistics.freight.closed_no_costs", &[])));
    }
    if amount <= 0 || description.trim().is_empty() {
        return Err(guard(english("logistics.freight.line_needed", &[])));
    }
    let mut line = JobLine {
        job_id,
        kind: line_kind::COST.to_owned(),
        description: description.trim().to_owned(),
        amount,
        source_ref: source_ref
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        late: closed_before(&state.db, job_id).await?,
        created_by: Some(user.id),
        ..JobLine::default()
    };
    line.insert(&state.db).await?;
    Ok(line)
}

/// `user` (`jobs.manage`) allocates the cost `line_id` with its liability and
/// whether it is rechargeable. A cost whose liability is the company is never
/// rechargeable.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a liability that is
/// not `client` or `company`, or a line that is not a cost.
pub async fn allocate(
    state: &AppState,
    user: &AuthUser,
    line_id: i64,
    liability_of: &str,
    rechargeable: bool,
) -> std::result::Result<JobLine, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let line = JobLine::find(&state.db, line_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if line.kind != line_kind::COST || !liability::ALL.contains(&liability_of) {
        return Err(guard(english("logistics.freight.liability_needed", &[])));
    }
    let job = job_of(&state.db, line.job_id).await?;
    if job.status == FreightStatus::Closed {
        return Err(guard(english("logistics.freight.closed_no_costs", &[])));
    }
    let recharge = rechargeable && liability_of == liability::CLIENT;
    let liability_value = Some(liability_of.to_owned());
    JobLine::query()
        .where_(JobLine::ID.eq(line_id))
        .update(
            &state.db,
            &[
                ("liability", &liability_value),
                ("allocated", &true),
                ("rechargeable", &recharge),
            ],
        )
        .await?;
    JobLine::find(&state.db, line_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`jobs.manage`) records a revenue line on the job `job_id`: what
/// the client is invoiced. `issued` says that its invoice is issued; the
/// invoice itself is `invoice_id` when there is one.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for an amount of 0 or
/// less, a blank description or a closed job.
pub async fn add_revenue(
    state: &AppState,
    user: &AuthUser,
    job_id: i64,
    description: &str,
    amount: i64,
    issued: bool,
    invoice_id: Option<i64>,
) -> std::result::Result<JobLine, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let job = job_of(&state.db, job_id).await?;
    if job.status == FreightStatus::Closed {
        return Err(guard(english("logistics.freight.closed_no_costs", &[])));
    }
    if amount <= 0 || description.trim().is_empty() {
        return Err(guard(english("logistics.freight.line_needed", &[])));
    }
    let mut line = JobLine {
        job_id,
        kind: line_kind::REVENUE.to_owned(),
        description: description.trim().to_owned(),
        amount,
        issued,
        invoice_id,
        late: closed_before(&state.db, job_id).await?,
        created_by: Some(user.id),
        ..JobLine::default()
    };
    line.insert(&state.db).await?;
    Ok(line)
}

/// Marks the revenue line `line_id` as issued (its invoice was issued).
///
/// # Errors
///
/// Database errors.
pub async fn mark_issued(db: &Db, line_id: i64, invoice_id: Option<i64>) -> Result {
    JobLine::query()
        .where_(JobLine::ID.eq(line_id))
        .update(db, &[("issued", &true), ("invoice_id", &invoice_id)])
        .await?;
    Ok(())
}

/// Writes the margin of the job `job_id` at this close, inside the close
/// transaction. Returns it.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn record_margin(
    tx: &mut renox::db::Transaction,
    job_id: i64,
    closed_by: Option<i64>,
) -> Result<FreightMargin> {
    let lines = lines_of(&mut *tx, job_id).await?;
    let figures = figures(&lines);
    let closes = FreightMargin::query()
        .where_(FreightMargin::JOB_ID.eq(job_id))
        .count(&mut *tx)
        .await?;
    let mut margin = FreightMargin {
        job_id,
        close_no: i64::try_from(closes).unwrap_or(i64::MAX) + 1,
        revenue: figures.revenue,
        cost: figures.cost,
        margin: figures.margin,
        closed_by,
        closed_at: renox::db::now(),
        ..FreightMargin::default()
    };
    margin.insert(&mut *tx).await?;
    Ok(margin)
}
