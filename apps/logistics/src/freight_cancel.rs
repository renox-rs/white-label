//! Cancelling a freight job before departure (03 Logistics §4.2 row *Client
//! cancels before departure*, §7.1 row `cancel`, §9 *VAT*; BL-LOG-23).
//!
//! - **The quotation** (`FIN.QUO.01`) is a seam. The cancellation charges are
//!   read through [`terms_of`]: the kernel's [`wl_kernel::quotations`]
//!   provider when the app has one, else the terms the documentation officer
//!   entered on the job ([`FreightQuotation`], [`set_quotation`]).
//! - **`cancel`** ([`cancel`]) takes a `booked` or `documents_pending` job
//!   with a reason; after departure it is not offered (the client's
//!   instruction is a hold). The stage is the job's state at `cancel`. When
//!   the quotation has a charge for the stage, one invoice is drafted with the
//!   cancellation charge, standard rated at 5% (no transport took place), and
//!   the rechargeable incurred costs when the quotation recharges them, each a
//!   reimbursement at 5% or, with the three conditions of VATP013 recorded, a
//!   disbursement `out_of_scope`. Without cancellation terms (or a charge for
//!   the stage) no charge is drafted and the documentation officers get a task
//!   to decide the incurred costs by hand. Incurred costs that are not
//!   recharged stay costs of the job: its loss, shown in
//!   [`FreightCancellation::absorbed`].
//! - **A cost billed after `cancel`** is allocated to the cancelled job (which
//!   stays `cancelled`); the accountants (`jobs.bill_cancelled`) are told and
//!   decide with [`decide_late_cost`] whether to invoice it or to absorb it.

use renox::auth::permissions::users_with_permission;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{self, DraftLine, InvoiceDraft, VatCategory};
use wl_kernel::quotations::{self, QuotationTerms};
use wl_kernel::role_tasks::{self, RoleTask};

use crate::freight::{FreightJob, FreightStatus, track};
use crate::freight_costing::{self, JobLine, line_kind};
use crate::freight_flow::{load, run, tell_client};
use crate::freight_machine::{CANCEL, FreightInput};
use crate::notices::tell_user;
use crate::permissions;
use crate::rates::aed;
use crate::texts::english;

/// What the accountant decided for a cost billed after the cancel.
pub mod decision {
    /// The cost is invoiced to the client.
    pub const INVOICED: &str = "invoiced";
    /// The cost stays with the company: the job's loss.
    pub const ABSORBED: &str = "absorbed";
}

/// The terms of the quotation as the documentation officer entered them: the
/// stand-in for the accepted quotation (`FIN.QUO.01`) of a job. Money is
/// fils.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_quotations", unique(job_id))]
pub struct FreightQuotation {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// The quotation's reference.
    pub reference: String,
    /// The cancellation charge for any stage.
    pub fixed_charge: Option<i64>,
    /// The cancellation charge at `booked`.
    pub booked_charge: Option<i64>,
    /// The cancellation charge at `documents_pending`.
    pub documents_pending_charge: Option<i64>,
    /// Whether the costs already incurred are recharged.
    pub recharge_incurred: bool,
    /// The user who entered the terms.
    pub entered_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl FreightQuotation {
    /// The terms in the shape the kernel's quotation seam answers.
    pub fn terms(&self) -> QuotationTerms {
        let mut terms = QuotationTerms::new(&self.reference);
        terms.cancellation_fixed = self.fixed_charge;
        if let Some(amount) = self.booked_charge {
            terms
                .cancellation_by_stage
                .push(("booked".to_owned(), amount));
        }
        if let Some(amount) = self.documents_pending_charge {
            terms
                .cancellation_by_stage
                .push(("documents_pending".to_owned(), amount));
        }
        terms.recharge_incurred = self.recharge_incurred;
        terms
    }
}

/// What a cancellation did.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_cancellations", unique(job_id))]
pub struct FreightCancellation {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// The job's state at `cancel`: `booked` or `documents_pending`.
    pub stage: String,
    /// Why it was cancelled.
    pub reason: String,
    /// Whether the quotation had a cancellation charge for the stage.
    pub terms_found: bool,
    /// The quotation's reference, when there was one.
    pub quotation_ref: String,
    /// The cancellation charge drafted, net, in fils.
    pub charge: i64,
    /// The incurred costs recharged on the invoice, net, in fils.
    pub recharged: i64,
    /// The incurred costs that were not recharged: the job's loss, in fils.
    pub absorbed: i64,
    /// The invoice that was drafted.
    pub invoice_id: Option<i64>,
    /// The task for the documentation officer, when nothing was drafted.
    pub ticket_id: Option<i64>,
    /// The user who cancelled the job.
    pub cancelled_by: Option<i64>,
    /// When it was cancelled.
    pub cancelled_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// The cancellation of the job, if it was cancelled.
///
/// # Errors
///
/// Database errors.
pub async fn cancellation_of(db: &Db, job_id: i64) -> Result<Option<FreightCancellation>> {
    FreightCancellation::query()
        .where_(FreightCancellation::JOB_ID.eq(job_id))
        .first(db)
        .await
}

/// What [`set_quotation`] takes.
#[derive(Debug, Clone, Default)]
pub struct QuotationInput {
    /// The quotation's reference.
    pub reference: String,
    /// The cancellation charge for any stage, in fils.
    pub fixed: Option<i64>,
    /// The cancellation charge at `booked`, in fils.
    pub booked: Option<i64>,
    /// The cancellation charge at `documents_pending`, in fils.
    pub documents_pending: Option<i64>,
    /// Whether the costs already incurred are recharged.
    pub recharge_incurred: bool,
}

/// `user` (`jobs.manage`) enters the terms of the accepted quotation of the
/// job `id` before it departs, replacing the ones there are.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a job that has
/// departed or a negative amount.
pub async fn set_quotation(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    input: &QuotationInput,
) -> std::result::Result<FreightQuotation, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let job = load(state, id).await?;
    if !matches!(
        job.status,
        FreightStatus::Booked | FreightStatus::DocumentsPending
    ) {
        return Err(guard(english("logistics.freight.quotation_departed", &[])));
    }
    let amounts = [input.fixed, input.booked, input.documents_pending];
    if amounts.iter().flatten().any(|amount| *amount < 0) {
        return Err(guard(english("logistics.freight.quotation_amount", &[])));
    }
    let reference = input.reference.trim().to_owned();
    let entered = Some(user.id);
    let known = FreightQuotation::query()
        .where_(FreightQuotation::JOB_ID.eq(id))
        .first(&state.db)
        .await?;
    if let Some(row) = known {
        FreightQuotation::query()
            .where_(FreightQuotation::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("reference", &reference),
                    ("fixed_charge", &input.fixed),
                    ("booked_charge", &input.booked),
                    ("documents_pending_charge", &input.documents_pending),
                    ("recharge_incurred", &input.recharge_incurred),
                    ("entered_by", &entered),
                ],
            )
            .await?;
    } else {
        let mut row = FreightQuotation {
            job_id: id,
            reference,
            fixed_charge: input.fixed,
            booked_charge: input.booked,
            documents_pending_charge: input.documents_pending,
            recharge_incurred: input.recharge_incurred,
            entered_by: entered,
            ..FreightQuotation::default()
        };
        row.insert(&state.db).await?;
    }
    FreightQuotation::query()
        .where_(FreightQuotation::JOB_ID.eq(id))
        .first(&state.db)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// The terms of the quotation of `job`: the kernel provider's answer when the
/// app has one and it knows the job, else the terms entered on the job, else
/// none.
///
/// # Errors
///
/// Database errors and the provider's.
pub async fn terms_of(state: &AppState, job: &FreightJob) -> Result<Option<QuotationTerms>> {
    if let Some(provider) = quotations::quotations(state) {
        let subject = job.subject();
        if let Some(terms) = provider.terms(&state.db, &subject).await? {
            return Ok(Some(terms));
        }
    }
    let row = FreightQuotation::query()
        .where_(FreightQuotation::JOB_ID.eq(job.id))
        .first(&state.db)
        .await?;
    Ok(row.map(|row| row.terms()))
}

/// `cancel` (`jobs.manage`) before departure with a reason; see the module
/// text for what is drafted.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `NotFrom` unless the job is `booked` or
/// `documents_pending`; `Guard` (422) without a reason.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<FreightJob, TransitionError> {
    let stage = load(state, id).await?.status.as_str().to_owned();
    let input = FreightInput {
        reason: Some(reason.trim().to_owned()),
        ..FreightInput::default()
    };
    let job = run(state, user, id, CANCEL, &input).await?;
    settle(state, user, &job, &stage, reason.trim()).await?;
    Ok(job)
}

/// Drafts what the quotation says for the cancelled `job` and tells the
/// client; opens the officer's task when nothing was drafted.
async fn settle(
    state: &AppState,
    user: &AuthUser,
    job: &FreightJob,
    stage: &str,
    reason: &str,
) -> std::result::Result<(), TransitionError> {
    let terms = terms_of(state, job).await?;
    let charge = terms
        .as_ref()
        .and_then(|terms| terms.cancellation_charge(stage));
    let recharge = terms.as_ref().is_some_and(|terms| terms.recharge_incurred);
    let lines = freight_costing::lines_of(&state.db, job.id).await?;
    let cost_total: i64 = lines
        .iter()
        .filter(|line| line.kind == line_kind::COST)
        .map(|line| line.amount)
        .sum();
    let incurred: Vec<&JobLine> = lines
        .iter()
        .filter(|line| {
            line.kind == line_kind::COST
                && line.allocated
                && line.rechargeable
                && line.invoice_id.is_none()
        })
        .collect();
    let mut invoice_id = None;
    let mut charged = 0;
    let mut recharged_cost = 0;
    if let Some(charge) = charge {
        let stage_word = stage.replace('_', " ");
        let text = english(
            "logistics.freight.cancellation_line",
            &[("number", &job.number), ("stage", &stage_word)],
        );
        let mut draft = InvoiceDraft::new(job.client(), job.subject())
            .line(DraftLine::new(&text, charge, VatCategory::Standard).charge_type("cancellation"));
        let recharged: Vec<&JobLine> = if recharge { incurred } else { Vec::new() };
        for line in &recharged {
            let text = english(
                "logistics.freight.cancellation_cost_line",
                &[("description", &line.description), ("number", &job.number)],
            );
            let (category, kind) = if line.disbursement {
                (VatCategory::OutOfScope, "disbursement")
            } else {
                (VatCategory::Standard, "reimbursement")
            };
            draft = draft.line(DraftLine::new(&text, line.amount, category).charge_type(kind));
        }
        let costs: i64 = recharged.iter().map(|line| line.amount).sum();
        let drafted = invoicing::invoicer(state).draft(&state.db, draft).await?;
        if let Some(invoice) = drafted {
            let description = english(
                "logistics.freight.cancellation_revenue",
                &[("number", &job.number)],
            );
            let mut revenue = JobLine {
                job_id: job.id,
                kind: line_kind::REVENUE.to_owned(),
                description,
                amount: charge + costs,
                issued: false,
                invoice_id: Some(invoice.0),
                created_by: Some(user.id),
                ..JobLine::default()
            };
            revenue.insert(&state.db).await?;
            let value = Some(invoice.0);
            for line in &recharged {
                JobLine::query()
                    .where_(JobLine::ID.eq(line.id))
                    .update(&state.db, &[("invoice_id", &value)])
                    .await?;
            }
            invoice_id = value;
            charged = charge;
            recharged_cost = costs;
        }
    }
    let incurred_open: i64 = lines
        .iter()
        .filter(|line| line.kind == line_kind::COST && line.invoice_id.is_none())
        .map(|line| line.amount)
        .sum();
    let mut ticket_id = None;
    if invoice_id.is_none() {
        let title = english(
            "logistics.freight.cancel_task_title",
            &[("number", &job.number)],
        );
        let description = english(
            "logistics.freight.cancel_task_body",
            &[("number", &job.number), ("incurred", &aed(incurred_open))],
        );
        let task = RoleTask::new(
            job.subject(),
            permissions::DOCUMENTATION_OFFICER_ROLE,
            &title,
            &format!("logistics.freight_cancel:{}", job.id),
        )
        .description(&description);
        ticket_id = role_tasks::open(state, task)
            .await?
            .map(|ticket| ticket.id.0);
    }
    let mut record = FreightCancellation {
        job_id: job.id,
        stage: stage.to_owned(),
        reason: reason.to_owned(),
        terms_found: charge.is_some(),
        quotation_ref: terms.map(|terms| terms.reference).unwrap_or_default(),
        charge: charged,
        recharged: recharged_cost,
        absorbed: cost_total - recharged_cost,
        invoice_id,
        ticket_id,
        cancelled_by: Some(user.id),
        cancelled_at: renox::db::now(),
        ..FreightCancellation::default()
    };
    record.insert(&state.db).await?;
    let note = format!(
        "{stage}: charge {}, recharged {}, absorbed {}",
        aed(record.charge),
        aed(record.recharged),
        aed(record.absorbed)
    );
    track(
        &state.db,
        job.id,
        "cancellation_settled",
        &note,
        Some(user.id),
    )
    .await?;
    let title = english(
        "logistics.freight.cancelled_title",
        &[("number", &job.number)],
    );
    let body = english(
        "logistics.freight.cancelled_body",
        &[("number", &job.number), ("reason", &reason)],
    );
    tell_client(state, job, &title, &body).await?;
    Ok(())
}

/// Tells the accountants (the holders of `jobs.bill_cancelled`) that the
/// cost `line` was recorded on the cancelled job: they decide whether to
/// invoice it.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn tell_accountants(state: &AppState, job: &FreightJob, line: &JobLine) -> Result {
    let title = english(
        "logistics.freight.late_cost_title",
        &[("number", &job.number)],
    );
    let body = english(
        "logistics.freight.late_cost_body",
        &[
            ("number", &job.number),
            ("description", &line.description),
            ("amount", &aed(line.amount)),
        ],
    );
    for user in users_with_permission(&state.db, permissions::JOBS_BILL_CANCELLED).await? {
        tell_user(state, job.subject(), &job.number, user.id, &title, &body).await?;
    }
    Ok(())
}

/// `user` (`jobs.bill_cancelled`) decides for the cost `line_id` of a
/// cancelled job: `invoice` it to the client (a draft invoice of that cost,
/// only when its liability is the client) or absorb it in the job's margin.
///
/// # Errors
///
/// `Forbidden` without `jobs.bill_cancelled`; `Guard` (422) unless the line is
/// a cost of a cancelled job with its liability and no decision yet, for a
/// company liability that would be invoiced, and when invoicing is off.
pub async fn decide_late_cost(
    state: &AppState,
    user: &AuthUser,
    line_id: i64,
    invoice: bool,
) -> std::result::Result<JobLine, TransitionError> {
    if !user.has_permission(permissions::JOBS_BILL_CANCELLED) {
        return Err(TransitionError::Forbidden);
    }
    let line = JobLine::find(&state.db, line_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    let job = load(state, line.job_id).await?;
    if line.kind != line_kind::COST || job.status != FreightStatus::Cancelled {
        return Err(guard(english("logistics.freight.not_cancelled_cost", &[])));
    }
    if line.decision.is_some() || line.invoice_id.is_some() {
        return Err(guard(english("logistics.freight.already_decided", &[])));
    }
    if !line.allocated {
        return Err(guard(english("logistics.freight.liability_first", &[])));
    }
    if !invoice {
        let chosen = Some(decision::ABSORBED.to_owned());
        JobLine::query()
            .where_(JobLine::ID.eq(line_id))
            .update(&state.db, &[("decision", &chosen)])
            .await?;
        track(
            &state.db,
            job.id,
            "late_cost_absorbed",
            &line.description,
            Some(user.id),
        )
        .await?;
        return JobLine::find(&state.db, line_id)
            .await?
            .ok_or(TransitionError::Failed(Error::NotFound));
    }
    if line.liability.as_deref() != Some(freight_costing::liability::CLIENT) {
        return Err(guard(english(
            "logistics.freight.company_not_recharged",
            &[],
        )));
    }
    let text = english(
        "logistics.freight.cancellation_cost_line",
        &[("description", &line.description), ("number", &job.number)],
    );
    let (category, kind) = if line.disbursement {
        (VatCategory::OutOfScope, "disbursement")
    } else {
        (VatCategory::Standard, "reimbursement")
    };
    let draft = InvoiceDraft::new(job.client(), job.subject())
        .line(DraftLine::new(&text, line.amount, category).charge_type(kind));
    let Some(invoice_ref) = invoicing::invoicer(state).draft(&state.db, draft).await? else {
        return Err(guard(english("logistics.freight.invoicing_off", &[])));
    };
    let description = english(
        "logistics.freight.cancellation_revenue",
        &[("number", &job.number)],
    );
    let mut revenue = JobLine {
        job_id: job.id,
        kind: line_kind::REVENUE.to_owned(),
        description,
        amount: line.amount,
        issued: false,
        invoice_id: Some(invoice_ref.0),
        late: true,
        created_by: Some(user.id),
        ..JobLine::default()
    };
    revenue.insert(&state.db).await?;
    let chosen = Some(decision::INVOICED.to_owned());
    let invoice_value = Some(invoice_ref.0);
    JobLine::query()
        .where_(JobLine::ID.eq(line_id))
        .update(
            &state.db,
            &[
                ("decision", &chosen),
                ("invoice_id", &invoice_value),
                ("rechargeable", &true),
            ],
        )
        .await?;
    track(
        &state.db,
        job.id,
        "late_cost_invoiced",
        &format!("{} / {}", invoice_ref.0, line.description),
        Some(user.id),
    )
    .await?;
    JobLine::find(&state.db, line_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}
