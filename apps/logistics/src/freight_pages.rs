//! The screens and actions of a freight job (03 Logistics §4.2, §7.1 *Freight
//! job*): `GET /freight/{id}` shows the job with its milestones, containers,
//! costs and margin to staff (`jobs.view`, `FIN.ACL.08`) and the status,
//! milestones, containers and the hold reason with its note to a client user
//! of the company (the portal screen proper is `POR.SLF.01`); every action is
//! a POST that answers 303 with a toast, 422 with the reason when refused.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::Serialize;

use crate::booking::{self, parse_dirhams};
use crate::freight::{FreightJob, FreightStatus, date_in};
use crate::freight_cancel::{self, QuotationInput};
use crate::freight_costing::{self, Figures};
use crate::freight_flow as flow;
use crate::freight_free_time::{self, BillCost};
use crate::freight_hold;
use crate::freight_machine as machine;
use crate::freight_reopen;
use crate::pages::{Fields, Texts, field, tx};
use crate::permissions;
use crate::texts::t;

/// The templates, compiled in.
pub(crate) const VIEWS: &[(&str, &str)] = &[(
    "logistics/freight_job.html",
    include_str!("../views/logistics/freight_job.html"),
)];

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/freight/{id}", show)
        .name("logistics.freight_job")
        .post("/freight/{id}/{action}", act)
        .name("logistics.freight_action")
        .post("/freight/{id}/costs", cost)
        .name("logistics.freight_cost")
        .post("/freight/{id}/revenue", revenue)
        .name("logistics.freight_revenue")
        .post("/freight/{id}/supplementary", supplementary)
        .name("logistics.freight_supplementary")
        .post("/freight/{id}/zero-rate-evidence", zero_rate_evidence)
        .name("logistics.freight_zero_rate_evidence")
        .post("/freight/{id}/quotation", quotation)
        .name("logistics.freight_quotation")
        .post("/freight/{id}/costs/bill", bill_cost)
        .name("logistics.freight_bill_cost")
        .post("/freight/{id}/recharge", recharge)
        .name("logistics.freight_recharge")
        .post("/freight/containers/{id}/collected", collected)
        .name("logistics.freight_collected")
        .post("/freight/containers/{id}/returned-empty", returned_empty)
        .name("logistics.freight_returned_empty")
        .post("/freight/containers/{id}/dates", end_dates)
        .name("logistics.freight_end_dates")
        .post("/freight/lines/{line}/decide", decide)
        .name("logistics.freight_decide")
        .post("/freight/lines/{line}/allocate", allocate)
        .name("logistics.freight_allocate")
        .post("/freight/lines/{line}/disbursement", disbursement)
        .name("logistics.freight_disbursement")
        .require_auth()
}

/// The badge colour of a status.
fn status_kind(status: FreightStatus) -> &'static str {
    match status {
        FreightStatus::Booked | FreightStatus::Cancelled => "info",
        FreightStatus::DocumentsPending | FreightStatus::OnHold => "warning",
        FreightStatus::InTransit
        | FreightStatus::CustomsClearance
        | FreightStatus::ReadyForDelivery => "info",
        FreightStatus::Delivered | FreightStatus::Closed => "success",
    }
}

/// The word of a status.
pub(crate) fn status_word(lang: &Lang, status: FreightStatus) -> String {
    t(
        lang,
        &format!("logistics.screen.job_status.{}", status.as_str()),
    )
}

/// One milestone or event of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct EventRow {
    kind: String,
    note: String,
    at: String,
}

/// One container of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ContainerRow {
    number: String,
    size: String,
    port_storage_end: String,
    demurrage_end: String,
    detention_end: String,
    collected_on: String,
    returned_empty_on: String,
}

/// One revenue or cost line of the page.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct LineRow {
    id: i64,
    kind: String,
    description: String,
    amount: i64,
    liability: String,
    allocated: bool,
    rechargeable: bool,
    issued: bool,
    late: bool,
    decision: String,
    reason: String,
    /// The days the log has next to the days the bill charges, for a
    /// container charge.
    days: String,
}

/// The margin at one close.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct MarginRow {
    close_no: i64,
    revenue: i64,
    cost: i64,
    margin: i64,
    at: String,
}

/// One input of an action.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FieldView {
    name: &'static str,
    label: String,
}

/// One action of the page the person may run.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct ActionView {
    slug: &'static str,
    label: String,
    url: String,
    fields: Vec<FieldView>,
}

/// The actions: the transition (the first of its twins), the path segment,
/// the text and the inputs.
const ACTIONS: &[(&str, &str, &str, &[&str])] = &[
    (
        machine::REQUEST_DOCUMENTS,
        "request-documents",
        "logistics.screen.job_request_documents",
        &["missing"],
    ),
    (
        machine::DEPART,
        "depart",
        "logistics.screen.job_depart",
        &["bl_awb"],
    ),
    (
        machine::ARRIVE,
        "arrive",
        "logistics.screen.job_arrive",
        &[],
    ),
    (
        machine::CLEAR,
        "clear",
        "logistics.screen.job_clear",
        &["declaration_no", "duties", "delivery_order"],
    ),
    (
        machine::HOLD,
        "hold",
        "logistics.screen.job_hold",
        &["reason", "note"],
    ),
    (
        machine::RELEASE_IN_TRANSIT,
        "release",
        "logistics.screen.job_release",
        &["note"],
    ),
    (
        machine::RELEASE_CUSTOMS_CLEARANCE,
        "release",
        "logistics.screen.job_release",
        &["note"],
    ),
    (
        machine::RELEASE_READY_FOR_DELIVERY,
        "release",
        "logistics.screen.job_release",
        &["note"],
    ),
    (
        machine::DELIVER,
        "deliver",
        "logistics.screen.job_deliver",
        &["pod_reference"],
    ),
    (machine::CLOSE, "close", "logistics.screen.job_close", &[]),
    (
        machine::REOPEN,
        "reopen",
        "logistics.screen.job_reopen",
        &["reason", "note"],
    ),
    (
        machine::CANCEL,
        "cancel",
        "logistics.screen.job_cancel",
        &["reason"],
    ),
];

/// What the job's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "logistics/freight_job.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    job: FreightJob,
    status_label: String,
    status_kind: &'static str,
    eta: String,
    /// The label of the hold reason, with `job.hold_note` shown beside it.
    hold_label: Option<String>,
    events: Vec<EventRow>,
    containers: Vec<ContainerRow>,
    staff: bool,
    lines: Vec<LineRow>,
    figures: Option<Figures>,
    margins: Vec<MarginRow>,
    actions: Vec<ActionView>,
    cancellation: Option<freight_cancel::FreightCancellation>,
    free_time: Vec<FreeTimeRow>,
}

/// One day of the free-time log.
#[derive(Serialize, Debug, Clone)]
pub(crate) struct FreeTimeRow {
    id: i64,
    container: String,
    charge: String,
    day: String,
    note: String,
}

fn date_text(date: Option<renox::chrono::NaiveDate>) -> String {
    date.map(|date| date.to_string()).unwrap_or_default()
}

/// `GET /freight/{id}`: staff with `jobs.view`, or a client user of the
/// job's company.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let job = FreightJob::find_or_404(&state.db, id).await?;
    let staff = user.has_permission(permissions::JOBS_VIEW);
    if !staff && !booking::sees_for(&state, &user, job.client()).await? {
        return Err(Error::Forbidden);
    }
    let offered: Vec<String> = if staff {
        job.available(&user, &state.db)
            .await?
            .into_iter()
            .map(|action| action.name)
            .collect()
    } else {
        Vec::new()
    };
    let actions = ACTIONS
        .iter()
        .filter(|(name, _, _, _)| offered.iter().any(|offered| offered.as_str() == *name))
        .map(|(_, slug, label, fields)| ActionView {
            slug: *slug,
            label: t(&lang, label),
            url: format!("/freight/{}/{slug}", job.id),
            fields: fields
                .iter()
                .map(|name| FieldView {
                    name: *name,
                    label: t(&lang, &format!("logistics.screen.job_field.{name}")),
                })
                .collect(),
        })
        .collect();
    let mut actions: Vec<ActionView> = actions;
    // The supplementary invoice of a reopened job is an action of its own.
    if staff
        && job.status == FreightStatus::Delivered
        && user.has_permission(permissions::JOBS_MANAGE)
        && !freight_costing::margins_of(&state.db, job.id)
            .await?
            .is_empty()
    {
        actions.push(ActionView {
            slug: "supplementary",
            label: t(&lang, "logistics.screen.job_supplementary"),
            url: format!("/freight/{}/supplementary", job.id),
            fields: Vec::new(),
        });
    }
    let hold_label = (job.status == FreightStatus::OnHold)
        .then(|| job.hold_reason.clone())
        .flatten()
        .map(|reason| freight_hold::reason_label(&lang, &reason));
    let lines = if staff {
        freight_costing::lines_of(&state.db, job.id).await?
    } else {
        Vec::new()
    };
    let figures = staff.then(|| freight_costing::figures(&lines));
    let margins = if staff {
        freight_costing::margins_of(&state.db, job.id).await?
    } else {
        Vec::new()
    };
    let mut days = std::collections::BTreeMap::new();
    for line in &lines {
        if let Some((logged, billed)) = freight_free_time::days_compared(&state.db, line).await? {
            days.insert(line.id, format!("{logged} / {billed}"));
        }
    }
    let containers = job.containers(&state.db).await?;
    let free_time: Vec<FreeTimeRow> = if staff {
        freight_free_time::log_of(&state.db, job.id)
            .await?
            .into_iter()
            .map(|entry| FreeTimeRow {
                id: entry.id,
                container: containers
                    .iter()
                    .find(|c| c.id == entry.container_id)
                    .map(|c| c.number.clone())
                    .unwrap_or_default(),
                charge: entry.charge_type.replace('_', " "),
                day: entry.log_date.to_string(),
                note: entry.note,
            })
            .collect()
    } else {
        Vec::new()
    };
    let cancellation = if staff {
        freight_cancel::cancellation_of(&state.db, job.id).await?
    } else {
        None
    };
    let page = ShowPage {
        cancellation,
        tx: tx(&lang),
        status_label: status_word(&lang, job.status),
        status_kind: status_kind(job.status),
        eta: date_text(job.eta),
        hold_label,
        events: job
            .events(&state.db)
            .await?
            .into_iter()
            .map(|e| EventRow {
                kind: e.kind,
                note: e.note,
                at: e.occurred_at.to_string(),
            })
            .collect(),
        free_time,
        containers: containers
            .into_iter()
            .map(|c| ContainerRow {
                number: c.number,
                size: c.size,
                port_storage_end: date_text(c.port_storage_end),
                demurrage_end: date_text(c.demurrage_end),
                detention_end: date_text(c.detention_end),
                collected_on: date_text(c.collected_on),
                returned_empty_on: date_text(c.returned_empty_on),
            })
            .collect(),
        staff,
        lines: lines
            .into_iter()
            .map(|l| LineRow {
                id: l.id,
                kind: l.kind,
                description: l.description,
                amount: l.amount,
                liability: l.liability.unwrap_or_default(),
                allocated: l.allocated,
                rechargeable: l.rechargeable,
                issued: l.issued,
                late: l.late,
                decision: l.decision.unwrap_or_default(),
                reason: l.liability_reason.unwrap_or_default(),
                days: days.get(&l.id).cloned().unwrap_or_default(),
            })
            .collect(),
        figures,
        margins: margins
            .into_iter()
            .map(|m| MarginRow {
                close_no: m.close_no,
                revenue: m.revenue,
                cost: m.cost,
                margin: m.margin,
                at: m.closed_at.to_string(),
            })
            .collect(),
        actions,
        job,
    };
    Ok(View::typed(page).into_response())
}

fn bad_input(lang: &Lang) -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        t(lang, "logistics.bad_input"),
    )
}

/// `POST /freight/{id}/{action}`: `request-documents` (`missing`), `depart`
/// (`bl_awb`), `arrive`, `clear` (`declaration_no`, `duties` in dirhams,
/// `delivery_order`), `hold` (`reason`, `note`), `hold-note` (`reason`, `note`: a second reason on
/// an open hold), `release` (`note`),
/// `deliver` (`pod_reference`), `close`, `reopen` (`reason`, `note`) and
/// `cancel` (`reason`).
async fn act(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, action)): Path<(i64, String)>,
    Form(form): Fields,
) -> Result<Response> {
    let done = match action.as_str() {
        "request-documents" => {
            flow::request_documents(&state, &user, id, &field(&form, "missing")).await
        }
        "depart" => flow::depart(&state, &user, id, &field(&form, "bl_awb")).await,
        "arrive" => flow::arrive(&state, &user, id).await,
        "clear" => {
            let text = field(&form, "duties");
            let duties = if text.trim().is_empty() {
                None
            } else {
                Some(parse_dirhams(&text).ok_or_else(|| bad_input(&lang))?)
            };
            flow::clear(
                &state,
                &user,
                id,
                &field(&form, "declaration_no"),
                duties,
                &field(&form, "delivery_order"),
            )
            .await
        }
        "hold" => {
            freight_hold::hold(
                &state,
                &user,
                id,
                &field(&form, "reason"),
                &field(&form, "note"),
            )
            .await
        }
        "hold-note" => {
            freight_hold::add_reason(
                &state,
                &user,
                id,
                &field(&form, "reason"),
                &field(&form, "note"),
            )
            .await
        }
        "release" => freight_hold::release(&state, &user, id, &field(&form, "note")).await,
        "reopen" => {
            freight_reopen::reopen(
                &state,
                &user,
                id,
                &field(&form, "reason"),
                &field(&form, "note"),
            )
            .await
        }
        "deliver" => flow::deliver(&state, &user, id, &field(&form, "pod_reference")).await,
        "close" => flow::close(&state, &user, id).await,
        "cancel" => flow::cancel(&state, &user, id, &field(&form, "reason")).await,
        _ => return Err(Error::NotFound),
    };
    done.map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/{id}/costs`: `description`, `amount` in dirhams and
/// `source_ref`.
async fn cost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let amount = parse_dirhams(&field(&form, "amount")).ok_or_else(|| bad_input(&lang))?;
    freight_costing::add_cost(
        &state,
        &user,
        id,
        &field(&form, "description"),
        amount,
        Some(field(&form, "source_ref").as_str()),
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/{id}/revenue`: `description`, `amount` in dirhams and
/// `issued` (`1` when the invoice is issued).
async fn revenue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let amount = parse_dirhams(&field(&form, "amount")).ok_or_else(|| bad_input(&lang))?;
    let issued = matches!(field(&form, "issued").trim(), "1" | "on" | "true");
    freight_costing::add_revenue(
        &state,
        &user,
        id,
        &field(&form, "description"),
        amount,
        issued,
        None,
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/lines/{line}/allocate`: `liability` (`client` or `company`),
/// its `reason` (late documents, late duty payment, late collection by the
/// client, company delay or other; needed for a container charge) and
/// `rechargeable` (`1`).
async fn allocate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(line): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let rechargeable = matches!(field(&form, "rechargeable").trim(), "1" | "on" | "true");
    let reason = field(&form, "reason");
    let allocated = freight_costing::allocate_with(
        &state,
        &user,
        line,
        field(&form, "liability").trim(),
        rechargeable,
        Some(reason.as_str()),
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&allocated.job_id])?,
    )
        .into_response())
}

/// A charge typed in dirhams; none when blank or zero.
fn charge_in(
    lang: &Lang,
    form: &std::collections::BTreeMap<String, String>,
    name: &str,
) -> Result<Option<i64>> {
    let text = field(form, name);
    if text.trim().is_empty() {
        return Ok(None);
    }
    let fils = parse_dirhams(&text).ok_or_else(|| bad_input(lang))?;
    Ok(Some(fils).filter(|fils| *fils > 0))
}

/// `POST /freight/{id}/quotation`: `reference`, the cancellation charges in
/// dirhams `fixed_charge`, `booked_charge` and `documents_pending_charge`
/// (blank for none) and `recharge_incurred` (`1`).
async fn quotation(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let input = QuotationInput {
        reference: field(&form, "reference"),
        fixed: charge_in(&lang, &form, "fixed_charge")?,
        booked: charge_in(&lang, &form, "booked_charge")?,
        documents_pending: charge_in(&lang, &form, "documents_pending_charge")?,
        recharge_incurred: matches!(
            field(&form, "recharge_incurred").trim(),
            "1" | "on" | "true"
        ),
    };
    freight_cancel::set_quotation(&state, &user, id, &input)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/lines/{line}/decide`: `invoice` (`1` to invoice the cost
/// billed after a cancel, `0` to absorb it).
async fn decide(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(line): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let invoice = matches!(field(&form, "invoice").trim(), "1" | "on" | "true");
    let decided = freight_cancel::decide_late_cost(&state, &user, line, invoice)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&decided.job_id])?,
    )
        .into_response())
}

/// A date typed in a form; today when blank; 422 when it is no date.
fn date_or_today(
    lang: &Lang,
    form: &std::collections::BTreeMap<String, String>,
    name: &str,
) -> Result<renox::chrono::NaiveDate> {
    let text = field(form, name);
    if text.trim().is_empty() {
        return Ok(wl_kernel::documents::today());
    }
    date_in(&text).ok_or_else(|| bad_input(lang))
}

/// `POST /freight/{id}/costs/bill`: a charge line of a supplier bill
/// (`bill_line_id`) allocated to the job and one of its containers
/// (`container`, `charge_type`, `billed_days`) with its `liability`, the
/// `reason` and `rechargeable` (`1`).
async fn bill_cost(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let number = |name: &str| -> Result<i64> {
        field(&form, name)
            .trim()
            .parse::<i64>()
            .map_err(|_| bad_input(&lang))
    };
    let input = BillCost {
        bill_line_id: number("bill_line_id")?,
        container: field(&form, "container"),
        charge_type: field(&form, "charge_type"),
        billed_days: number("billed_days")?,
        liability: field(&form, "liability"),
        reason: field(&form, "reason"),
        rechargeable: matches!(field(&form, "rechargeable").trim(), "1" | "on" | "true"),
    };
    freight_free_time::add_bill_cost(&state, &user, id, &input)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/{id}/recharge`: drafts and issues the invoice of the
/// rechargeable costs of the open job.
async fn recharge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    freight_reopen::recharge_invoice(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/containers/{id}/collected`: `date` (today when blank).
async fn collected(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let on = date_or_today(&lang, &form, "date")?;
    let container = freight_free_time::record_collected(&state, &user, id, on)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&container.job_id])?,
    )
        .into_response())
}

/// `POST /freight/containers/{id}/returned-empty`: `date` (today when blank).
async fn returned_empty(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let on = date_or_today(&lang, &form, "date")?;
    let container = freight_free_time::record_returned_empty(&state, &user, id, on)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&container.job_id])?,
    )
        .into_response())
}

/// `POST /freight/containers/{id}/dates`: the new `port_storage_end`,
/// `demurrage_end` and `detention_end` (a blank one is left as it is).
async fn end_dates(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let container = freight_free_time::set_end_dates(
        &state,
        &user,
        id,
        [
            date_in(&field(&form, "port_storage_end")),
            date_in(&field(&form, "demurrage_end")),
            date_in(&field(&form, "detention_end")),
        ],
    )
    .await
    .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&container.job_id])?,
    )
        .into_response())
}

/// `POST /freight/{id}/supplementary`: drafts and issues the supplementary
/// invoice of the late costs that are rechargeable.
async fn supplementary(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    freight_reopen::supplementary_invoice(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/{id}/zero-rate-evidence`: `attached` (`1` or `0`).
async fn zero_rate_evidence(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let attached = matches!(field(&form, "attached").trim(), "1" | "on" | "true");
    freight_reopen::set_zero_rate_evidence(&state, &user, id, attached)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&id])?,
    )
        .into_response())
}

/// `POST /freight/lines/{line}/disbursement`: `conditions` (`1` when the
/// three conditions of VATP013 are recorded for the cost).
async fn disbursement(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(line): Path<i64>,
    Form(form): Fields,
) -> Result<Response> {
    let met = matches!(field(&form, "conditions").trim(), "1" | "on" | "true");
    let line = freight_reopen::set_disbursement(&state, &user, line, met)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(t(&lang, "logistics.freight_done")),
        Redirect::route("logistics.freight_job", &[&line.job_id])?,
    )
        .into_response())
}
