//! The record page of a client contract (`GET /manpower/contracts/{id}`,
//! BL-MPW-22, `docs/UX.md` §2.2): title, status badge, the actions this person
//! may run now (a blocked one disabled with the guard in words), and the tabs
//! *Overview*, *Deployments*, *Rate cards*, *Approvals* and *Activity*. The
//! client's sites are a block of the overview.
//!
//! The page only reads; every button posts to the route of the action
//! (`pages.rs`), which runs the existing service. A refusal comes back as the
//! service's 422 and shows in the sheet that sent the form.
//!
//! Decisions where the spec is silent:
//!
//! - *Mark signed* takes the signed document as the reference the contract
//!   keeps (`signed_document`, a file name or a document number); the page
//!   shows it as text. Linking it waits for the documents module's file page.
//! - *Cancel* sits last with the danger style and asks in its own sheet (the
//!   confirmation) instead of under a *More* menu: the kit's menu cannot open a
//!   sheet.
//! - A pending version dated by a renewal is listed in the *Rate cards* tab
//!   with its status; the renewal itself needs the version accepted (BL-MPW-05).

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, awaiting::action_label, decide};
use wl_kernel::documents::today;
use wl_kernel::rate_cards::rate_cards;

use super::{
    Act, Texts, aed, date_or, date_text, party_text, period_text, role_text, transition_text,
    user_text,
};
use crate::contract::{ClientContract, ContractStatus, RELEASE, RELEASE_TERMINATING, TERMINATE};
use crate::contracts;
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments::worker_name;
use crate::hold::{ContractHold, HoldReason};
use crate::permissions;
use crate::site::Site;
use crate::texts::{screen_texts, t, tf, word};

/// The permission to see rate cards (wl-invoicing `rate_cards.view`, `MPW.ACL.04`).
const RATE_CARDS_VIEW: &str = "rate_cards.view";
/// The permission to make rate card versions (wl-invoicing `rate_cards.manage`).
const RATE_CARDS_MANAGE: &str = "rate_cards.manage";

/// The address of a contract's page.
pub(crate) fn url(id: i64) -> String {
    format!("/manpower/contracts/{id}")
}

/// One fact of the overview.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Fact {
    label: String,
    value: String,
}

/// The credit hold of a contract in words.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct HoldView {
    since: String,
    before: String,
    by: String,
    /// One line per open reason, with its date.
    reasons: Vec<String>,
}

/// What the person may do with the contract now.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ContractActions {
    sign: Act,
    activate: Act,
    cancel: Act,
    hold: Act,
    release: Act,
    terminate: Act,
    renew: Act,
    terms: Act,
}

/// A deployment of the contract in the *Deployments* tab.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct DeploymentLine {
    url: String,
    worker: String,
    role: String,
    site: String,
    period: String,
    status: String,
    status_kind: &'static str,
}

/// A rate card version in the *Rate cards* tab.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct VersionLine {
    /// The page of the version, empty for a person who may not see rates.
    url: String,
    version: String,
    effective: String,
    status: String,
    status_kind: &'static str,
    accepted: String,
    lines: i64,
}

/// An approval request of the contract in the *Approvals* tab.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ApprovalLine {
    id: i64,
    title: String,
    status: String,
    status_kind: &'static str,
    requester: String,
    requested: String,
    decided: String,
    waiting: bool,
    can_decide: bool,
}

/// A client site in the *Sites* block.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct SiteLine {
    id: i64,
    name: String,
    address: String,
    licences: String,
    action: String,
}

/// A row of the *Activity* tab.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ActivityLine {
    at: String,
    what: String,
    by: String,
    reason: String,
}

/// What the contract page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/contract.html")]
pub(crate) struct ContractPage {
    tx: Texts,
    id: i64,
    reference: String,
    client: String,
    client_party_id: i64,
    subtitle: String,
    status: String,
    status_key: String,
    status_kind: &'static str,
    facts: Vec<Fact>,
    /// The checks that keep a draft from being activated, in words.
    checks: Vec<String>,
    hold: Option<HoldView>,
    /// *Waiting for the MD: ...* lines.
    waiting: Vec<String>,
    actions: ContractActions,
    approvals: Vec<ApprovalLine>,
    deployments: Vec<DeploymentLine>,
    versions: Vec<VersionLine>,
    /// The rate card page, empty when the contract has no card or the person
    /// may not see rates (`MPW.ACL.04`).
    rate_card_url: String,
    /// Whether *Start the rate card* is offered.
    can_start_rate_card: bool,
    subject: String,
    sites: Vec<SiteLine>,
    can_sites: bool,
    activity: Vec<ActivityLine>,
    hold_reasons: Vec<(String, String)>,
    hold_selected: String,
    initiators: Vec<(String, String)>,
    initiator_selected: String,
    today: String,
}

/// The badge tone of a contract status.
pub(crate) fn status_kind(status: ContractStatus) -> &'static str {
    match status {
        ContractStatus::Active => "success",
        ContractStatus::OnHold | ContractStatus::Terminating => "warning",
        ContractStatus::Draft | ContractStatus::Ended | ContractStatus::Cancelled => "info",
    }
}

/// The badge tone of a deployment status.
fn deployment_kind(status: DeploymentStatus) -> &'static str {
    match status {
        DeploymentStatus::Active => "success",
        DeploymentStatus::Suspended => "warning",
        DeploymentStatus::Planned | DeploymentStatus::Ended | DeploymentStatus::Cancelled => "info",
    }
}

/// The badge tone of a rate card version status.
fn version_kind(status: &str) -> &'static str {
    match status {
        "accepted" => "success",
        "rejected" => "error",
        "pending" | "awaiting_approval" => "warning",
        _ => "info",
    }
}

/// The badge tone of an approval request status.
fn approval_kind(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Approved => "success",
        ApprovalStatus::Rejected | ApprovalStatus::Failed => "error",
        ApprovalStatus::Pending | ApprovalStatus::Returned => "warning",
        ApprovalStatus::Withdrawn | ApprovalStatus::Expired => "info",
    }
}

fn fact(label: String, value: String) -> Fact {
    Fact { label, value }
}

/// The text of a hold reason with its cause, the amount and the date it was
/// added: *Bounced cheque CHQ-9, AED 15,000.00, since 05 Nov 2026*.
fn hold_line(lang: &Lang, row: &ContractHold) -> String {
    let what = row.label.clone().unwrap_or_default();
    let amount = if row.amount > 0 {
        aed(row.amount)
    } else {
        String::new()
    };
    let since = row
        .created_at
        .map(|at| date_text(wl_kernel::documents::dubai_date(at)))
        .unwrap_or_default();
    let cause = match row.reason {
        HoldReason::Overdue => tf(
            lang,
            "manpower.screen.hold.overdue",
            &[("what", &what), ("amount", &amount)],
        ),
        HoldReason::BouncedCheque => tf(
            lang,
            "manpower.screen.hold.cheque",
            &[("what", &what), ("amount", &amount)],
        ),
        HoldReason::Other => tf(
            lang,
            "manpower.screen.hold.other",
            &[("text", &row.reason_text.clone().unwrap_or_default())],
        ),
    };
    tf(
        lang,
        "manpower.screen.hold.line",
        &[("cause", &cause.trim()), ("since", &since)],
    )
}

/// The checks that keep draft `contract` from being activated today, in words:
/// the signed contract, a rate card version accepted by the client.
pub(crate) async fn activation_checks(
    state: &AppState,
    contract: &ClientContract,
) -> Result<Vec<String>> {
    contracts::readiness(state, contract, today()).await
}

/// `GET /manpower/contracts/{id}`: the page for a browser, the same data as
/// JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let contract = ClientContract::find_or_404(&state.db, id).await?;
    let scope = contract.scope();
    let manage = user.has_permission_in(permissions::CONTRACTS_MANAGE, &scope);
    if !manage && !user.has_permission_in(permissions::CONTRACTS_VIEW, &scope) {
        return Err(Error::Forbidden);
    }
    let page = build(&state, &user, &lang, &contract, manage).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

/// The page of `contract` for `user`.
async fn build(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    contract: &ClientContract,
    manage: bool,
) -> Result<ContractPage> {
    let not_set = t(lang, "manpower.screen.not_set");
    let client = party_text(state, lang, Some(contract.client_party_id), &not_set).await;
    let status = contract.status;
    let pending: Vec<ApprovalRequest> =
        ApprovalRequest::pending_about(&state.db, &contract.subject()).await?;
    let release_waits = pending
        .iter()
        .any(|request| request.transition == RELEASE || request.transition == RELEASE_TERMINATING);
    let terminate_waits = pending
        .iter()
        .any(|request| request.transition == TERMINATE);

    // The overview facts.
    let mut facts = vec![
        fact(
            t(lang, "manpower.screen.contract.period"),
            period_text(lang, contract.start_date, Some(contract.end_date)),
        ),
        fact(
            t(lang, "manpower.screen.contracts.billing"),
            word(lang, "billing", contract.billing_cycle.as_str()),
        ),
        fact(
            t(lang, "manpower.screen.contract.payment_terms"),
            tf(
                lang,
                "manpower.screen.contract.days",
                &[("days", &contract.payment_terms_days)],
            ),
        ),
        fact(
            t(lang, "manpower.screen.contracts.cover"),
            if contract.cover_required {
                t(lang, "manpower.screen.contracts.cover_required")
            } else {
                t(lang, "manpower.screen.contracts.cover_none")
            },
        ),
        fact(
            t(lang, "manpower.screen.contract.notice"),
            tf(
                lang,
                "manpower.screen.contract.days",
                &[("days", &contract.notice_period_days)],
            ),
        ),
        fact(
            t(lang, "manpower.screen.contract.deemed"),
            contract.deemed_approval_days.map_or_else(
                || t(lang, "manpower.screen.contract.no_deemed"),
                |days| tf(lang, "manpower.screen.contract.days", &[("days", &days)]),
            ),
        ),
        fact(
            t(lang, "manpower.screen.contract.escalation"),
            party_text(state, lang, contract.escalation_contact_party_id, &not_set).await,
        ),
        fact(
            t(lang, "manpower.screen.contract.manager"),
            user_text(state, contract.account_manager_user_id, &not_set).await,
        ),
    ];
    facts.push(fact(
        t(lang, "manpower.screen.contract.signed"),
        match contract.signed_on {
            Some(on) => tf(
                lang,
                "manpower.screen.contract.signed_on",
                &[("date", &date_text(on))],
            ),
            None => t(lang, "manpower.screen.contract.not_signed"),
        },
    ));
    facts.push(fact(
        t(lang, "manpower.screen.contract.document"),
        contract
            .signed_document
            .clone()
            .filter(|document| !document.trim().is_empty())
            .unwrap_or_else(|| t(lang, "manpower.screen.contract.no_document")),
    ));
    if let Some(on) = contract.activated_on {
        facts.push(fact(
            t(lang, "manpower.screen.contract.activated"),
            date_text(on),
        ));
    }
    if let Some(reason) = contract
        .cancel_reason
        .as_ref()
        .filter(|r| !r.trim().is_empty())
    {
        facts.push(fact(
            t(lang, "manpower.screen.contract.cancel_reason"),
            reason.clone(),
        ));
    }
    if status == ContractStatus::Terminating {
        let by = contract
            .termination_initiated_by
            .clone()
            .unwrap_or_default();
        facts.push(fact(
            t(lang, "manpower.screen.contract.termination"),
            tf(
                lang,
                "manpower.screen.contract.terminating_line",
                &[
                    ("by", &word(lang, "initiator", &by)),
                    (
                        "reason",
                        &contract.termination_reason.clone().unwrap_or_default(),
                    ),
                    ("notice", &date_or(contract.termination_notice_on, &not_set)),
                    ("was", &date_or(contract.original_end_date, &not_set)),
                ],
            ),
        ));
    }
    if let Some(at) = contract.renewed_at {
        facts.push(fact(
            t(lang, "manpower.screen.contract.renewal"),
            tf(
                lang,
                "manpower.screen.contract.renewed_line",
                &[
                    ("end", &date_text(contract.end_date)),
                    (
                        "was",
                        &date_or(contract.renewal_previous_end_date, &not_set),
                    ),
                    ("on", &date_text(wl_kernel::documents::dubai_date(at))),
                    ("rates", &date_or(contract.renewal_rates_from, &not_set)),
                ],
            ),
        ));
    }

    // The checks before an activation.
    let checks = if status == ContractStatus::Draft && manage {
        activation_checks(state, contract).await?
    } else {
        Vec::new()
    };

    // The credit hold.
    let hold = if status == ContractStatus::OnHold {
        let open = ContractHold::open_of(&state.db, contract.id).await?;
        Some(HoldView {
            since: date_or(
                contract.held_at.map(wl_kernel::documents::dubai_date),
                &not_set,
            ),
            before: contract.status_before_hold.map_or_else(
                || not_set.clone(),
                |before| word(lang, "status", before.as_str()),
            ),
            by: match contract.held_by {
                Some(id) => user_text(state, Some(id), &not_set).await,
                None => t(lang, "manpower.screen.hold.by_rule"),
            },
            reasons: open.iter().map(|row| hold_line(lang, row)).collect(),
        })
    } else {
        None
    };

    // The actions.
    let live = matches!(
        status,
        ContractStatus::Active | ContractStatus::Terminating | ContractStatus::OnHold
    );
    let actions = ContractActions {
        sign: Act::when(manage && status == ContractStatus::Draft),
        activate: if manage && status == ContractStatus::Draft {
            Act::unless((!checks.is_empty()).then(|| checks.join(" ")))
        } else {
            Act::hidden()
        },
        cancel: Act::when(manage && status == ContractStatus::Draft),
        hold: Act::when(manage && live),
        release: Act::when(manage && status == ContractStatus::OnHold && !release_waits),
        terminate: Act::when(
            manage
                && matches!(status, ContractStatus::Active | ContractStatus::OnHold)
                && !terminate_waits,
        ),
        renew: Act::when(
            manage && matches!(status, ContractStatus::Active | ContractStatus::OnHold),
        ),
        terms: Act::when(
            manage && !matches!(status, ContractStatus::Ended | ContractStatus::Cancelled),
        ),
    };

    // The approvals of the contract, newest first.
    let requests = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq(contract.subject().subject_type.as_str()))
        .where_(ApprovalRequest::SUBJECT_ID.eq(contract.id))
        .order_by_desc(ApprovalRequest::ID)
        .get(&state.db)
        .await?;
    let mut approvals = Vec::new();
    let mut waiting = Vec::new();
    for request in &requests {
        let title = action_label(lang, &request.action);
        let is_waiting = request.is_pending();
        if is_waiting {
            waiting.push(tf(
                lang,
                "manpower.screen.approval.waiting_for",
                &[("action", &title)],
            ));
        }
        approvals.push(ApprovalLine {
            id: request.id,
            title,
            status: if is_waiting {
                t(lang, "manpower.screen.approval.waiting")
            } else {
                word(lang, "approval", &request.status.to_string())
            },
            status_kind: approval_kind(request.status),
            requester: user_text(state, Some(request.requester_id), &not_set).await,
            requested: date_or(
                request.created_at.map(wl_kernel::documents::dubai_date),
                &not_set,
            ),
            decided: date_or(
                request.decided_at.map(wl_kernel::documents::dubai_date),
                &not_set,
            ),
            waiting: is_waiting,
            can_decide: is_waiting
                && decide::ensure_approver(&state.db, request, user)
                    .await
                    .is_ok(),
        });
    }

    // The deployments the person may see.
    let mut deployments = Vec::new();
    for deployment in Deployment::query()
        .where_(Deployment::CLIENT_CONTRACT_ID.eq(contract.id))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?
    {
        let may = [
            permissions::DEPLOYMENTS_VIEW,
            permissions::DEPLOYMENTS_MANAGE,
        ]
        .iter()
        .any(|permission| {
            user.has_permission_in(permission, &deployment.scope())
                || user.has_permission_in(permission, &deployment.site_scope())
        });
        if !may {
            continue;
        }
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        deployments.push(DeploymentLine {
            url: format!("/manpower/deployments/{}", deployment.id),
            worker: worker_name(state, &lang.locale, deployment.employee()).await,
            role: role_text(&deployment.role),
            site,
            period: period_text(lang, deployment.start_date, deployment.end_date),
            status: word(lang, "status", deployment.status.as_str()),
            status_kind: deployment_kind(deployment.status),
        });
    }

    // The rate card versions: where each stands, never the rates; the rates
    // are on the card pages of those who may see them.
    let can_rates = user.has_permission_anywhere(RATE_CARDS_VIEW);
    let cards = rate_cards(state);
    let card = cards.card_of(state, &contract.subject()).await?;
    let rows = cards.versions(state, &contract.subject()).await?;
    let rate_card_url = match card {
        Some(id) if can_rates => format!("/rate-cards/{id}"),
        _ => String::new(),
    };
    let can_start_rate_card =
        card.is_none() && manage && user.has_permission_anywhere(RATE_CARDS_MANAGE);
    let versions = rows
        .into_iter()
        .map(|row| VersionLine {
            url: if can_rates {
                format!("/rate-card-versions/{}", row.id)
            } else {
                String::new()
            },
            version: format!("v{}", row.version),
            effective: date_text(row.effective_from),
            status: word(lang, "rate_status", &row.status),
            status_kind: version_kind(&row.status),
            accepted: row.accepted_on.map_or_else(
                || not_set.clone(),
                |on| {
                    let on = date_text(on);
                    match row.accepted_document.filter(|d| !d.trim().is_empty()) {
                        Some(document) => format!("{on}, {document}"),
                        None => on,
                    }
                },
            ),
            lines: row.lines,
        })
        .collect();

    // The client's sites.
    let can_sites = user.has_permission_in(permissions::SITES_MANAGE, &contract.scope());
    let sites = Site::query()
        .where_(Site::CLIENT_PARTY_ID.eq(contract.client_party_id))
        .order_by(Site::ID)
        .get(&state.db)
        .await?
        .into_iter()
        .map(|site| SiteLine {
            action: format!("/manpower/sites/{}", site.id),
            id: site.id,
            name: site.name,
            address: site.address.unwrap_or_else(|| not_set.clone()),
            licences: site
                .required_licences
                .filter(|licences| !licences.trim().is_empty())
                .unwrap_or_else(|| t(lang, "manpower.screen.contract.no_licences")),
        })
        .collect();

    // The activity: every change of state, oldest first.
    let mut activity = Vec::new();
    for row in renox::state_machine::history(&state.db, ClientContract::TABLE, contract.id).await? {
        activity.push(ActivityLine {
            at: row
                .at
                .with_timezone(&wl_kernel::BusinessCalendar::offset())
                .format("%d %b %Y %H:%M")
                .to_string(),
            what: transition_text(lang, &row.transition),
            by: match row.actor_id {
                Some(id) => user_text(state, Some(id), &not_set).await,
                None => t(lang, "manpower.screen.activity.system"),
            },
            reason: row.reason.unwrap_or_default(),
        });
    }

    Ok(ContractPage {
        tx: screen_texts(lang),
        id: contract.id,
        reference: contract.reference.clone(),
        client: client.clone(),
        client_party_id: contract.client_party_id,
        subtitle: format!(
            "{client}, {}",
            period_text(lang, contract.start_date, Some(contract.end_date))
        ),
        status: word(lang, "status", status.as_str()),
        status_key: status.as_str().to_owned(),
        status_kind: status_kind(status),
        facts,
        checks,
        hold,
        waiting,
        actions,
        approvals,
        deployments,
        versions,
        rate_card_url,
        can_start_rate_card,
        subject: contract.subject().to_string(),
        sites,
        can_sites,
        activity,
        hold_reasons: super::options(lang, "hold_reason", &["overdue", "bounced_cheque", "other"]),
        hold_selected: "bounced_cheque".to_owned(),
        initiators: super::options(lang, "initiator", &["client", "agency"]),
        initiator_selected: "client".to_owned(),
        today: today().to_string(),
    })
}
