//! The service contract page (wl-ticketing §4.6, §5.6, issue #149):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /service-contracts/{id}` | `wl_ticketing.contract` | `service_contracts.view` |
//! | `POST /service-contracts/{id}/activate` | `wl_ticketing.contract_activate` | `service_contracts.manage` |
//! | `POST /service-contracts/{id}/terminate` | `wl_ticketing.contract_terminate` | `service_contracts.manage` (fields `reason`, `termination_date`) |
//! | `POST /service-contracts/{id}/visits/{ticket}/reassign` | `wl_ticketing.contract_visit_reassign` | `tickets.assign` (fields `assignee_user_id` or `party_code`, `reason_code`, `reason_text`) |
//! | `POST /service-contracts/{id}/visits/{ticket}/cancel` | `wl_ticketing.contract_visit_cancel` | `tickets.assign` (field `reason`) |
//!
//! The page shows the contract, what it covers, its planned visits and the
//! compliance flags raised for it. It is also the coordinator's task when the
//! contract ends (`WRK.RUL.18`, [`crate::contract_end`]): each open visit has
//! "Reassign" (the reason `contract_ended` unless another is chosen) and, for a
//! category that is not *reassign only*, "Cancel" with a reason. A cancel of a
//! reassign-only category's visit from here is refused (422). The template is
//! compiled in and can be replaced by the app
//! (`resources/views/wl-ticketing/contract.html`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::parties::directory;
use wl_kernel::tickets::Assignee;

use crate::assign::{AssignRequest, assign, reasons_of};
use crate::config::Category;
use crate::contract::{ContractStatus, ServiceContract, ServiceContracts, is_open_visit};
use crate::contract_end::CONTRACT_ENDED_REASON;
use crate::flags::ComplianceFlag;
use crate::no_show::NoShow;
use crate::pages::{Texts, assignee_name, invalid, party_by_code, when};
use crate::permissions;
use crate::texts;
use crate::ticket::{Ticket, TicketStatus};
use crate::visibility;
use crate::work;

/// The routes, each behind its permission.
pub fn routes() -> Routes {
    Routes::new()
        .get("/service-contracts/{id}", show)
        .name("wl_ticketing.contract")
        .require_permission(permissions::SERVICE_CONTRACTS_VIEW)
        .merge(
            Routes::new()
                .post("/service-contracts/{id}/activate", store_activate)
                .name("wl_ticketing.contract_activate")
                .post("/service-contracts/{id}/terminate", store_terminate)
                .name("wl_ticketing.contract_terminate")
                .require_permission(permissions::SERVICE_CONTRACTS_MANAGE),
        )
        .merge(
            Routes::new()
                .post(
                    "/service-contracts/{id}/visits/{ticket}/reassign",
                    store_reassign,
                )
                .name("wl_ticketing.contract_visit_reassign")
                .post(
                    "/service-contracts/{id}/visits/{ticket}/cancel",
                    store_cancel,
                )
                .name("wl_ticketing.contract_visit_cancel")
                .require_permission(permissions::ASSIGN),
        )
}

/// The badge colour of a contract status.
fn status_kind(status: ContractStatus) -> &'static str {
    match status {
        ContractStatus::Active | ContractStatus::Renewed => "success",
        ContractStatus::Expiring => "warning",
        ContractStatus::Draft | ContractStatus::Expired | ContractStatus::Terminated => "info",
    }
}

/// The contract as the card shows it.
#[derive(Serialize, Debug, Clone)]
pub struct ContractCard {
    /// The id.
    pub id: i64,
    /// The number.
    pub number: String,
    /// The title.
    pub title: String,
    /// The vendor's name.
    pub vendor: String,
    /// The first day it holds.
    pub start: String,
    /// The last day it holds.
    pub end: String,
    /// The days before the end date it becomes `expiring`.
    pub notice_days: i64,
    /// Whether a covered category is compliance-critical.
    pub compliance: bool,
    /// The reason and date of a termination, e.g. `Vendor closed (30 Nov 2026)`.
    pub termination: String,
    /// When the end was handled (`WRK.RUL.18`), if it was.
    pub ended: String,
}

/// One covered subject and category.
#[derive(Serialize, Debug, Clone)]
pub struct CoverageRow {
    /// `property.building_system #4`.
    pub subject: String,
    /// The category's name.
    pub category: String,
}

/// One planned visit, with the actions the user is offered.
#[derive(Serialize, Debug, Clone)]
pub struct VisitRow {
    /// The ticket id.
    pub id: i64,
    /// The ticket number.
    pub number: String,
    /// The title.
    pub title: String,
    /// The planned due date.
    pub planned_due: String,
    /// The ticket status, as a label.
    pub status: String,
    /// The vendor or staff member it is assigned to.
    pub assignee: String,
    /// Whether "Reassign" is offered.
    pub can_reassign: bool,
    /// Whether "Cancel" is offered: not for a reassign-only category.
    pub can_cancel: bool,
    /// The reason the reassign sheet starts with.
    pub reason_code: String,
}

/// One compliance flag raised for the contract.
#[derive(Serialize, Debug, Clone)]
pub struct ContractFlagRow {
    /// `property.building_system #4`.
    pub subject: String,
    /// The category's name.
    pub category: String,
    /// The reason's label.
    pub reason: String,
    /// When it was raised.
    pub raised: String,
}

/// The vendor no-shows of one vendor under the contract.
#[derive(Serialize, Debug, Clone)]
pub struct NoShowRow {
    /// The vendor's name.
    pub vendor: String,
    /// How many no-shows.
    pub count: i64,
    /// The latest scheduled time that was missed.
    pub last: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-ticketing/contract.html")]
pub(crate) struct ContractPage {
    tx: Texts,
    contract: ContractCard,
    status_label: String,
    status_kind: &'static str,
    can_activate: bool,
    can_terminate: bool,
    coverage: Vec<CoverageRow>,
    visits: Vec<VisitRow>,
    flags: Vec<ContractFlagRow>,
    /// The no-shows per vendor.
    no_shows: Vec<NoShowRow>,
    /// `(id, name)` of the staff a visit can be reassigned to.
    users: Vec<(i64, String)>,
    /// `(code, name)` of the reassignment reasons.
    reasons: Vec<(String, String)>,
    /// Whether any visit has an action: the actions column is worth its space.
    has_actions: bool,
}

/// A date as the page writes it: `30 Nov 2026`.
fn day(date: NaiveDate) -> String {
    date.format("%d %b %Y").to_string()
}

/// The reassign sheet's reason for `visit`: the contract's end when it has
/// ended, else none chosen.
fn default_reason(contract: &ServiceContract, _visit: &Ticket) -> String {
    if contract.ended_at.is_some() {
        CONTRACT_ENDED_REASON.to_owned()
    } else {
        String::new()
    }
}

/// `GET /service-contracts/{id}`: the contract, its coverage, visits and
/// flags.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(contract): Found<ServiceContract>,
) -> Result<View> {
    let db = state.db.clone();
    let t = |key: &str| texts::t(&lang, &format!("wl_ticketing.{key}"));
    let vendor = directory(&state)
        .display_name(&db, contract.vendor(), "en")
        .await?
        .unwrap_or_else(|| format!("#{}", contract.vendor_party_id));

    let mut coverage = Vec::new();
    for row in contract.coverage(&db).await? {
        let category = Category::find(&db, row.category_id)
            .await?
            .map(|c| c.name(&lang.locale).to_owned())
            .unwrap_or_default();
        coverage.push(CoverageRow {
            subject: format!("{} #{}", row.subject_type, row.subject_id),
            category,
        });
    }

    let assigner = user.has_permission(permissions::ASSIGN);
    let mut visits = Vec::new();
    for visit in contract.visits(&db).await? {
        let category = Category::find(&db, visit.category_id).await?;
        let assignee = match visit.assignee() {
            Some(a) => assignee_name(&state, a).await?,
            None => String::new(),
        };
        let open = is_open_visit(&visit) && visibility::ensure_visible(&visit, &user).is_ok();
        let reassign_only = category.as_ref().is_some_and(|c| c.contract_end_reassign_only);
        visits.push(VisitRow {
            id: visit.id,
            number: visit.number.clone(),
            title: visit.title.clone(),
            planned_due: visit.planned_due_date.map(day).unwrap_or_default(),
            status: t(&format!("status.{}", visit.status.as_str())),
            assignee,
            can_reassign: assigner && open,
            can_cancel: assigner && open && !reassign_only,
            reason_code: default_reason(&contract, &visit),
        });
    }

    let mut flags = Vec::new();
    let open_flags = ComplianceFlag::query()
        .where_(ComplianceFlag::CONTRACT_ID.eq(contract.id))
        .where_(ComplianceFlag::CLEARED_AT.is_null())
        .order_by(ComplianceFlag::RAISED_AT)
        .get(&db)
        .await?;
    for flag in open_flags {
        let category = Category::find(&db, flag.category_id)
            .await?
            .map(|c| c.name(&lang.locale).to_owned())
            .unwrap_or_default();
        flags.push(ContractFlagRow {
            subject: format!("{} #{}", flag.subject_type, flag.subject_id),
            category,
            reason: t(&format!("flags.reason_{}", flag.reason.as_str())),
            raised: when(&state, Some(flag.raised_at)).unwrap_or_default(),
        });
    }

    let mut per_vendor: std::collections::BTreeMap<i64, (i64, renox::db::DateTime)> =
        std::collections::BTreeMap::new();
    for row in NoShow::of_contract(&db, contract.id).await? {
        let entry = per_vendor
            .entry(row.vendor_party_id)
            .or_insert((0, row.scheduled_at));
        entry.0 += 1;
        if row.scheduled_at > entry.1 {
            entry.1 = row.scheduled_at;
        }
    }
    let mut no_shows = Vec::new();
    for (vendor, (count, last)) in per_vendor {
        let name = directory(&state)
            .display_name(&db, wl_kernel::PartyId(vendor), "en")
            .await?
            .unwrap_or_else(|| format!("#{vendor}"));
        no_shows.push(NoShowRow {
            vendor: name,
            count,
            last: when(&state, Some(last)).unwrap_or_default(),
        });
    }
    let manage = user.has_permission(permissions::SERVICE_CONTRACTS_MANAGE);
    let users = if assigner && visits.iter().any(|v| v.can_reassign) {
        let mut users: Vec<(i64, String)> = User::query()
            .get(&db)
            .await?
            .into_iter()
            .map(|u| (u.id, u.name))
            .collect();
        users.sort_by(|a, b| a.1.cmp(&b.1));
        users
    } else {
        Vec::new()
    };
    let reasons = reasons_of(&state)
        .all()
        .iter()
        .map(|r| (r.code.clone(), r.name.clone()))
        .collect();
    let termination = match (&contract.termination_reason, contract.termination_date) {
        (Some(reason), Some(date)) => format!("{reason} ({})", day(date)),
        _ => String::new(),
    };
    let card = ContractCard {
        id: contract.id,
        number: contract.number.clone(),
        title: contract.title.clone(),
        vendor,
        start: day(contract.start_date),
        end: day(contract.end_date),
        notice_days: contract.renewal_notice_days,
        compliance: contract.compliance_contract,
        termination,
        ended: when(&state, contract.ended_at).unwrap_or_default(),
    };
    let has_actions = visits.iter().any(|v| v.can_reassign || v.can_cancel);
    Ok(View::typed(ContractPage {
        tx: texts::all(&lang),
        status_label: t(&format!("contract.status_{}", contract.status.as_str())),
        status_kind: status_kind(contract.status),
        can_activate: manage && contract.status == ContractStatus::Draft,
        can_terminate: manage
            && matches!(
                contract.status,
                ContractStatus::Active | ContractStatus::Expiring
            ),
        contract: card,
        coverage,
        visits,
        flags,
        no_shows,
        users,
        reasons,
        has_actions,
    }))
}

/// `POST /service-contracts/{id}/activate`: activates the draft contract.
async fn store_activate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(contract): Found<ServiceContract>,
) -> Result<Response> {
    ServiceContracts::activate(&state, &user, &contract)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((
        Toast::success(texts::t(&lang, "wl_ticketing.contract.activated")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The terminate sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct TerminateForm {
    /// Why the contract ends early.
    #[validate(required, max = 500)]
    pub reason: String,
    /// The day it ends: it covers its subjects until then.
    pub termination_date: NaiveDate,
}

/// `POST /service-contracts/{id}/terminate`: terminates the contract; the
/// open visits are handled on the termination date (at once when it is today).
async fn store_terminate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Found(contract): Found<ServiceContract>,
    Valid(form): Valid<TerminateForm>,
) -> Result<Response> {
    let reason = form.reason.trim().to_owned();
    match ServiceContracts::terminate(&state, &user, &contract, &reason, form.termination_date)
        .await
    {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_ticketing.contract.terminated")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::ReasonRequired) => Err(invalid(
            "reason",
            texts::t(&lang, "wl_ticketing.contract.reason_required"),
            &form,
        )),
        Err(TransitionError::Guard(message)) => Err(invalid("termination_date", message, &form)),
        Err(other) => Err(other.into_error()),
    }
}

/// The ticket `ticket` of the visits of `contract`, or 404.
async fn visit_of(state: &AppState, contract: &ServiceContract, ticket: i64) -> Result<Ticket> {
    let visit = Ticket::find_or_404(&state.db, ticket).await?;
    if visit.contract_id != Some(contract.id) || visit.planned_due_date.is_none() {
        return Err(Error::NotFound);
    }
    Ok(visit)
}

/// The reassign sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct ReassignForm {
    /// The staff user who gets the visit.
    pub assignee_user_id: Option<i64>,
    /// The code of the vendor party that gets it.
    #[validate(max = 32)]
    pub party_code: Option<String>,
    /// The reassignment reason; `contract_ended` when left out.
    #[validate(max = 64)]
    pub reason_code: Option<String>,
    /// The explanation given with the reason.
    #[validate(max = 2000)]
    pub reason_text: Option<String>,
}

/// `POST /service-contracts/{id}/visits/{ticket}/reassign`: reassigns the
/// visit to another vendor or to staff, with the reason `contract_ended`
/// unless another is given.
async fn store_reassign(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, ticket)): Path<(i64, i64)>,
    Valid(form): Valid<ReassignForm>,
) -> Result<Response> {
    let contract = ServiceContract::find_or_404(&state.db, id).await?;
    let visit = visit_of(&state, &contract, ticket).await?;
    visibility::ensure_visible(&visit, &user)?;
    let t = |key: &str| texts::t(&lang, &format!("wl_ticketing.{key}"));
    let party = form.party_code.as_deref().filter(|c| !c.trim().is_empty());
    let assignee = match (form.assignee_user_id, party) {
        (Some(_), Some(_)) => return Err(invalid("party_code", t("assign.both"), &form)),
        (Some(id), None) => Assignee::User { id },
        (None, Some(code)) => match party_by_code(&state, code).await? {
            Some(id) => Assignee::Party { id },
            None => return Err(invalid("party_code", t("assign.unknown_party"), &form)),
        },
        (None, None) => return Err(invalid("assignee_user_id", t("assign.choose"), &form)),
    };
    let reason = form
        .reason_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(CONTRACT_ENDED_REASON)
        .to_owned();
    let request = AssignRequest {
        assignee,
        priority: visit.priority,
        visit_at: visit.visit_at,
        reason_code: Some(reason),
        reason_text: form.reason_text.clone(),
    };
    match assign(&state, &user, &visit, request, renox::db::now()).await {
        Ok(_) => Ok((Toast::success(t("assign.done")), renox::HxRefresh).into_response()),
        Err(TransitionError::Guard(message)) => {
            let field = if message.contains("reason") || message.contains("explanation") {
                "reason_code"
            } else {
                "party_code"
            };
            Err(invalid(field, message, &form))
        }
        Err(other) => Err(other.into_error()),
    }
}

/// The cancel sheet's form.
#[derive(Deserialize, Serialize, Validate, Debug)]
pub struct CancelForm {
    /// Why the visit is cancelled.
    #[validate(required, max = 500)]
    pub reason: String,
}

/// `POST /service-contracts/{id}/visits/{ticket}/cancel`: cancels the visit
/// with a reason; refused (422) for a category set to reassign only.
async fn store_cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((id, ticket)): Path<(i64, i64)>,
    Valid(form): Valid<CancelForm>,
) -> Result<Response> {
    let contract = ServiceContract::find_or_404(&state.db, id).await?;
    let visit = visit_of(&state, &contract, ticket).await?;
    visibility::ensure_visible(&visit, &user)?;
    let reassign_only = Category::find(&state.db, visit.category_id)
        .await?
        .is_some_and(|c| c.contract_end_reassign_only);
    if reassign_only {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::t(&lang, "wl_ticketing.contract.reassign_only"),
        ));
    }
    if visit.status == TicketStatus::InProgress {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::t(&lang, "wl_ticketing.contract.started"),
        ));
    }
    match work::cancel(&state, &user, &visit, form.reason.trim()).await {
        Ok(()) => Ok((
            Toast::success(texts::t(&lang, "wl_ticketing.cancel_ticket.done")),
            renox::HxRefresh,
        )
            .into_response()),
        Err(TransitionError::ReasonRequired) => Err(invalid(
            "reason",
            texts::t(&lang, "wl_ticketing.cancel_ticket.required"),
            &form,
        )),
        Err(other) => Err(other.into_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contract_status_has_a_colour() {
        assert_eq!(status_kind(ContractStatus::Active), "success");
        assert_eq!(status_kind(ContractStatus::Expiring), "warning");
        assert_eq!(status_kind(ContractStatus::Terminated), "info");
        assert_eq!(status_kind(ContractStatus::Draft), "info");
    }

    #[test]
    fn a_date_reads_day_month_year() {
        assert_eq!(day(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()), "30 Nov 2026");
    }

    #[test]
    fn the_reassign_sheet_starts_with_the_contract_end_once_it_is_handled() {
        let mut contract = ServiceContract::default();
        let visit = Ticket::default();
        assert_eq!(default_reason(&contract, &visit), "");
        contract.ended_at = Some(renox::db::now());
        assert_eq!(default_reason(&contract, &visit), "contract_ended");
    }
}
