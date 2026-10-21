//! Reopening a closed freight job for a late cost (03 Logistics §4.2 row
//! *Supplier bill arrives after the job is closed*, §7.1 rows `close` and
//! `reopen`, §9 *VAT* and *Disbursements*; BL-LOG-22).
//!
//! - **Reopen** ([`reopen`]): `jobs.close` reopens a `closed` job with a
//!   reason ([`reason`]: late supplier bill, demurrage, port storage, or
//!   *other* with a note); the job is `delivered` again. A cancelled job is
//!   never reopened and a reopening without a reason is refused. The
//!   [`FreightReopening`] keeps the reason and the margin of the close it
//!   reopened; the first invoice is never changed.
//! - **The late cost** is recorded and allocated as any cost
//!   ([`crate::freight_costing`]), now flagged `late`, with its liability:
//!   *company* only lowers the margin, *client* may be rechargeable.
//! - **The supplementary invoice** ([`supplementary_invoice`]): a new invoice
//!   to the client for the late costs that are rechargeable, drafted through
//!   the kernel's invoicer and issued by the user. Each cost line is a
//!   disbursement, `out_of_scope`, when the three conditions of VATP013 are
//!   recorded ([`set_disbursement`], `FIN.DSB.01`); otherwise a reimbursement
//!   with the VAT of the job's main service: 5%, or zero-rated only for a job
//!   marked international with the evidence attached
//!   ([`set_zero_rate_evidence`]). Its revenue is a revenue line of the job,
//!   which `close` requires to be issued.
//! - **Close again** is the same `close` with the same guards: the margin is
//!   recalculated and the earlier margin stays in the history
//!   ([`crate::freight_costing::margins_of`]).

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::{self, DraftLine, InvoiceDraft, InvoiceId, VatCategory};

use crate::freight::{FreightJob, FreightStatus, track};
use crate::freight_costing::{
    self, FreightMargin, JobLine, line_kind, margins_of, mark_issued,
};
use crate::freight_flow::{load, run};
use crate::freight_machine::{FreightInput, REOPEN, given, input_of, needs, trimmed};
use crate::permissions;
use crate::texts::english;

/// Why a closed job is reopened.
pub mod reason {
    /// A supplier bill that arrived after the close.
    pub const LATE_SUPPLIER_BILL: &str = "late_supplier_bill";
    /// Demurrage billed after the close.
    pub const DEMURRAGE: &str = "demurrage";
    /// Port storage billed after the close.
    pub const PORT_STORAGE: &str = "port_storage";
    /// Anything else; a note says what.
    pub const OTHER: &str = "other";
    /// Every reason.
    pub const ALL: [&str; 4] = [LATE_SUPPLIER_BILL, DEMURRAGE, PORT_STORAGE, OTHER];
}

/// One reopening of a closed job.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_freight_reopenings", index(job_id))]
pub struct FreightReopening {
    /// The primary key.
    pub id: i64,
    /// The job.
    #[model(references = "logistics_freight_jobs")]
    pub job_id: i64,
    /// One of [`reason`].
    pub reason: String,
    /// The note, needed for *other*.
    pub note: String,
    /// The margin of the close it reopened, in fils.
    pub margin_before: i64,
    /// The user who reopened the job.
    pub reopened_by: Option<i64>,
    /// When it was reopened.
    pub reopened_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The `reopen` transition.
pub(crate) fn reopen_transition() -> Transition<FreightJob> {
    let transition = Transition::new(REOPEN)
        .label("Reopen")
        .tone("warning")
        .permission(permissions::JOBS_CLOSE)
        .from([FreightStatus::Closed])
        .to(FreightStatus::Delivered);
    let transition = needs(
        transition,
        "reason_given",
        "logistics.freight.reopen_reason_needed",
        |_job, input| {
            input
                .reason
                .as_deref()
                .is_some_and(|reason| reason::ALL.contains(&reason.trim()))
        },
    );
    needs(
        transition,
        "note_given_for_other",
        "logistics.freight.reopen_note_needed",
        |_job, input| input.reason.as_deref().map(str::trim) != Some(reason::OTHER) || given(&input.note),
    )
    .effect(|job: &FreightJob, ctx, tx| {
        let id = job.id;
        let input = input_of(ctx);
        let actor = ctx.actor().map(|user| user.id);
        Box::pin(async move {
            let why = trimmed(&input.reason).unwrap_or_default();
            let note = trimmed(&input.note).unwrap_or_default();
            let last: Option<FreightMargin> = FreightMargin::query()
                .where_(FreightMargin::JOB_ID.eq(id))
                .order_by_desc(FreightMargin::CLOSE_NO)
                .first(&mut *tx)
                .await?;
            let mut row = FreightReopening {
                job_id: id,
                reason: why.clone(),
                note: note.clone(),
                margin_before: last.map_or(0, |margin| margin.margin),
                reopened_by: actor,
                reopened_at: renox::db::now(),
                ..FreightReopening::default()
            };
            row.insert(&mut *tx).await?;
            track(&mut *tx, id, "reopened", &format!("{why}: {note}"), actor).await
        })
    })
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// The reopenings of the job, the first first.
///
/// # Errors
///
/// Database errors.
pub async fn reopenings_of(db: &Db, job_id: i64) -> Result<Vec<FreightReopening>> {
    FreightReopening::query()
        .where_(FreightReopening::JOB_ID.eq(job_id))
        .order_by(FreightReopening::ID)
        .get(db)
        .await
}

/// `user` (`jobs.close`) reopens the closed job `id` for a late cost, with a
/// `why` (one of [`reason`]) and a `note` (needed for *other*). The job is
/// `delivered` again; the margin is recalculated at the next `close`.
///
/// # Errors
///
/// `Forbidden` without `jobs.close`; `NotFrom` unless the job is `closed`
/// (a cancelled job is never reopened); `Guard` (422) without a known reason
/// or for *other* without a note.
pub async fn reopen(
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
    run(state, user, id, REOPEN, &input).await
}

/// `user` (`jobs.manage`) records that the three conditions of VATP013 are met
/// (or are not) for the cost `line_id`: it is then billed as a disbursement,
/// `out_of_scope`.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a line that is not a
/// cost or is already on an invoice.
pub async fn set_disbursement(
    state: &AppState,
    user: &AuthUser,
    line_id: i64,
    conditions_met: bool,
) -> std::result::Result<JobLine, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let line = JobLine::find(&state.db, line_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if line.kind != line_kind::COST || line.invoice_id.is_some() {
        return Err(guard(english("logistics.freight.liability_needed", &[])));
    }
    JobLine::query()
        .where_(JobLine::ID.eq(line_id))
        .update(&state.db, &[("disbursement", &conditions_met)])
        .await?;
    JobLine::find(&state.db, line_id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`jobs.manage`) records that the evidence for zero-rating is
/// attached to the job `id` (or is not).
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`.
pub async fn set_zero_rate_evidence(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    attached: bool,
) -> std::result::Result<FreightJob, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    FreightJob::query()
        .where_(FreightJob::ID.eq(id))
        .update(&state.db, &[("zero_rate_evidence", &attached)])
        .await?;
    load(state, id).await
}

/// The VAT treatment of the recharge of `line` on the job `job` (§9): a
/// disbursement is `out_of_scope`; any other recharge is a reimbursement with
/// the VAT of the job's main service, 5%, or zero-rated only for a job marked
/// international with its evidence attached.
pub fn vat_category(job: &FreightJob, line: &JobLine) -> VatCategory {
    if line.disbursement {
        VatCategory::OutOfScope
    } else if job.international && job.zero_rate_evidence {
        VatCategory::ZeroRated
    } else {
        VatCategory::Standard
    }
}

/// What [`supplementary_invoice`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supplementary {
    /// The invoice.
    pub invoice: InvoiceId,
    /// The late costs on it.
    pub lines: usize,
    /// Their net total, in fils.
    pub net: i64,
    /// Whether the invoice is issued.
    pub issued: bool,
}

/// `user` (`jobs.manage`) drafts and issues the supplementary invoice of the
/// late costs of the reopened job `id` that are rechargeable (liability
/// *client*, recharged) and not yet on an invoice. The invoice references the
/// job and the first invoice in its line texts; the first invoice is not
/// changed. Its net total becomes a revenue line of the job; the costs are
/// marked as recharged on it.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) unless the job is
/// `delivered` after a reopening, when there is no rechargeable late cost and
/// when invoicing is off; the invoicer's refusal to issue (the draft and the
/// revenue line stay, unissued, and `close` waits for them).
pub async fn supplementary_invoice(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Supplementary, TransitionError> {
    invoice_recharges(state, user, id, true).await
}

/// `user` (`jobs.manage`) drafts and issues the invoice of the costs of the
/// open job `id` (not closed, not cancelled) that are rechargeable (liability
/// *client*, recharged, for example the demurrage the shipping line billed
/// for a late collection, BL-LOG-24) and not yet on an invoice. It is the same
/// invoice as [`supplementary_invoice`] with the costs recorded before the
/// first close.
///
/// # Errors
///
/// `Forbidden` without `jobs.manage`; `Guard` (422) for a job that is closed
/// or cancelled, when there is no rechargeable cost and when invoicing is off.
pub async fn recharge_invoice(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Supplementary, TransitionError> {
    invoice_recharges(state, user, id, false).await
}

/// The invoice of the rechargeable costs that are `late` (recorded after a
/// close, on a reopened job) or not (on an open job).
async fn invoice_recharges(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    late: bool,
) -> std::result::Result<Supplementary, TransitionError> {
    if !user.has_permission(permissions::JOBS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let job = load(state, id).await?;
    if late {
        if job.status != FreightStatus::Delivered || margins_of(&state.db, id).await?.is_empty() {
            return Err(guard(english("logistics.freight.not_reopened", &[])));
        }
    } else if matches!(job.status, FreightStatus::Closed | FreightStatus::Cancelled) {
        return Err(guard(english("logistics.freight.not_open", &[])));
    }
    let lines = freight_costing::lines_of(&state.db, id).await?;
    // A draft whose issue was refused (the user may not issue, the entity is
    // not registered...) is issued once the cause is cleared: no second draft.
    let waiting = lines.iter().find(|line| {
        line.kind == line_kind::REVENUE
            && line.late == late
            && !line.issued
            && line.invoice_id.is_some()
    });
    if let Some(waiting) = waiting {
        let invoice = InvoiceId(waiting.invoice_id.unwrap_or_default());
        invoicing::invoicer(state)
            .issue(state, invoice, Some(user))
            .await?;
        mark_issued(&state.db, waiting.id, Some(invoice.0)).await?;
        let recharged = lines
            .iter()
            .filter(|line| line.kind == line_kind::COST && line.invoice_id == Some(invoice.0))
            .count();
        return Ok(Supplementary {
            invoice,
            lines: recharged,
            net: waiting.amount,
            issued: true,
        });
    }
    let costs: Vec<&JobLine> = lines
        .iter()
        .filter(|line| {
            line.kind == line_kind::COST
                && line.late == late
                && line.rechargeable
                && line.invoice_id.is_none()
        })
        .collect();
    if costs.is_empty() {
        return Err(guard(english("logistics.freight.nothing_to_recharge", &[])));
    }
    // The first invoice: the earliest revenue line that has one.
    let first = lines
        .iter()
        .find(|line| line.kind == line_kind::REVENUE && !line.late && line.invoice_id.is_some())
        .and_then(|line| line.invoice_id)
        .filter(|_| late);
    let first_text = first.map_or_else(String::new, |invoice| format!(" / invoice {invoice}"));
    let mut draft = InvoiceDraft::new(job.client(), job.subject());
    for line in &costs {
        let category = vat_category(&job, line);
        let text = english(
            "logistics.freight.supplementary_line",
            &[
                ("description", &line.description),
                ("number", &job.number),
                ("first", &first_text),
            ],
        );
        let kind = if line.disbursement {
            "disbursement"
        } else {
            "reimbursement"
        };
        draft = draft.line(DraftLine::new(&text, line.amount, category).charge_type(kind));
    }
    let net: i64 = costs.iter().map(|line| line.amount).sum();
    let invoicer = invoicing::invoicer(state);
    let Some(invoice) = invoicer.draft(&state.db, draft).await? else {
        return Err(guard(english("logistics.freight.invoicing_off", &[])));
    };
    let description = english(
        if late {
            "logistics.freight.supplementary_revenue"
        } else {
            "logistics.freight.recharge_revenue"
        },
        &[("number", &job.number)],
    );
    let mut revenue = JobLine {
        job_id: id,
        kind: line_kind::REVENUE.to_owned(),
        description,
        amount: net,
        issued: false,
        invoice_id: Some(invoice.0),
        late,
        created_by: Some(user.id),
        ..JobLine::default()
    };
    revenue.insert(&state.db).await?;
    let invoice_value = Some(invoice.0);
    for line in &costs {
        JobLine::query()
            .where_(JobLine::ID.eq(line.id))
            .update(&state.db, &[("invoice_id", &invoice_value)])
            .await?;
    }
    track(
        &state.db,
        id,
        if late { "supplementary_invoice" } else { "recharge_invoice" },
        &format!("{} / {net}", invoice.0),
        Some(user.id),
    )
    .await?;
    invoicer.issue(state, invoice, Some(user)).await?;
    mark_issued(&state.db, revenue.id, Some(invoice.0)).await?;
    Ok(Supplementary {
        invoice,
        lines: costs.len(),
        net,
        issued: true,
    })
}
