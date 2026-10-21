//! Non-standard exits (wl-hr §4 *Employee*, §6 *Access rules* `HRM.ACL.07`,
//! issue #194): `cancel_onboarding`, `withdraw_notice`, `report_absconding`,
//! `reinstate` and `record_death` change an employee's status outside the
//! normal path. HR (`employees.manage`) requests one with a reason and the
//! documents its guard names; another user with `employees.exit.approve`
//! (default: the MD) approves it; only then the transition runs.
//!
//! - **One approval per action.** [`policies`] is the default policy of each
//!   action key `hr.<transition>`: one step, held by `employees.exit.approve`,
//!   with segregation of duties on, so the requester never decides, not even
//!   as an admin or the MD. When the MD requests, another holder of the
//!   permission must approve; with none the request waits.
//! - **The request.** [`request`] checks the reason and the documents first
//!   ([`missing`]; a request without them is refused and creates no approval
//!   request), refuses a second request while one is pending for the employee
//!   (the kernel only refuses the same action twice, so the five actions are
//!   checked together here), then runs the user transition as the requester.
//!   The kernel's interceptor defers it: the employee keeps its state and the
//!   approvers are told. The page shows the pending request as a badge.
//! - **The approval.** At the last yes the kernel runs `<name>_approved` as the
//!   system with the request's payload ([`ExitInput`]) as input. The guard is
//!   the same one, so the documents are checked again at the approval; a
//!   failed check ends the request `failed` and nothing runs.
//! - **Audit.** The request, each decision and the transition are written to
//!   the audit log against the employee, with the reason and the attached
//!   documents (action `hr.exit_requested`, `hr.exit_approved` and
//!   `hr.exit_transition`; an end without a yes is `hr.exit_<end>`). The
//!   history row of the transition carries the requester and the approver
//!   in its metadata.
//!
//! The checks that read the database and the effects of each exit live with the
//! exit: `report_absconding` in [`crate::absconding`] (#197), `cancel_onboarding`,
//! `withdraw_notice` and `reinstate` in [`crate::exit_effects`] (#195, #196,
//! #198) and `record_death` in [`crate::death`] (#199).

use renox::auth::permissions::users_with_permission_in;
use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalPolicy, ApprovalRequest, PolicyDefault};
use wl_kernel::attendance::attendance_history;
use wl_kernel::documents::today;
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_kernel::{ApprovalDecided, ApprovalEnd};

use crate::absconding;
use crate::assignees;
use crate::config::config_of;
use crate::employee::Employee;
use crate::employee_machine::{
    CANCEL_ONBOARDING, RECORD_DEATH, REINSTATE, REPORT_ABSCONDING, WITHDRAW_NOTICE,
};
use crate::exit_effects;
use crate::permissions;
use crate::texts::english;

/// The five transitions that need a second user's approval.
pub const EXIT_TRANSITIONS: [&str; 5] = [
    CANCEL_ONBOARDING,
    WITHDRAW_NOTICE,
    REPORT_ABSCONDING,
    REINSTATE,
    RECORD_DEATH,
];

/// The document kind of the other party's written agreement to a withdrawn
/// notice.
pub const KIND_AGREEMENT: &str = "withdrawal_agreement";
/// The document kind of a death certificate.
pub const KIND_DEATH_CERTIFICATE: &str = "death_certificate";
/// The document kind of an authority's notice of death.
pub const KIND_AUTHORITY_NOTICE: &str = "authority_notice";

/// The audit action of a request.
pub const AUDIT_REQUESTED: &str = "hr.exit_requested";
/// The audit action of the final approval.
pub const AUDIT_APPROVED: &str = "hr.exit_approved";
/// The audit action of the transition that the approval ran.
pub const AUDIT_TRANSITION: &str = "hr.exit_transition";

/// Whether `transition` is one of the five that need approval.
pub fn is_exit(transition: &str) -> bool {
    EXIT_TRANSITIONS.contains(&transition)
}

/// The approval policies' action key of `transition`: `hr.<transition>`.
pub fn action_key(transition: &str) -> String {
    format!("hr.{transition}")
}

/// The default policy of each action: one step by `employees.exit.approve`,
/// segregation of duties on, no expiry (a request waits for the MD).
pub fn policies() -> Vec<PolicyDefault> {
    EXIT_TRANSITIONS
        .iter()
        .map(|name| {
            PolicyDefault::new(action_key(name))
                .step_permission(permissions::EMPLOYEES_EXIT_APPROVE)
        })
        .collect()
}

/// A document attached to a request: a stored document, a reference, or both.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct ExitDocument {
    /// The kind, e.g. [`KIND_DEATH_CERTIFICATE`].
    pub kind: String,
    /// The stored document (wl-documents), when there is a file.
    #[serde(default)]
    pub document: Option<i64>,
    /// The number or reference written on it, when given.
    #[serde(default)]
    pub reference: Option<String>,
}

impl ExitDocument {
    /// Whether the document is given: a file or a non-blank reference.
    pub fn is_given(&self) -> bool {
        self.document.is_some()
            || self
                .reference
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty())
    }
}

/// What a request carries; it is the transition's input and the payload
/// snapshot of the approval request, so the guard sees it again at the
/// approval.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ExitInput {
    /// Why. Always required.
    pub reason: String,
    /// The documents attached.
    pub documents: Vec<ExitDocument>,
    /// The MoHRE absence report number (`report_absconding`).
    pub report_number: Option<String>,
    /// The MoHRE absence report date (`report_absconding`).
    pub report_date: Option<NaiveDate>,
    /// The report file, when available (`report_absconding`): a stored
    /// document.
    pub report_file: Option<i64>,
    /// The date of death (`record_death`).
    pub death_date: Option<NaiveDate>,
    /// The date the worker returned (`reinstate`).
    pub return_date: Option<NaiveDate>,
    /// The authority's reference of the withdrawal or cancellation of the
    /// absence report (`reinstate`).
    pub withdrawal_reference: Option<String>,
    /// Whether the employee has attendance (`cancel_onboarding`): read when
    /// the request was made, because the checks at the approval have only the
    /// database. The request overwrites whatever the form sent.
    pub has_attendance: Option<bool>,
    /// Absence days before `report_absconding` is allowed: the setting when
    /// the request was made (the checks at the approval cannot read it).
    pub absence_days: Option<i64>,
}

fn blank(text: &Option<String>) -> bool {
    text.as_deref().is_none_or(|t| t.trim().is_empty())
}

impl ExitInput {
    /// Whether a document of `kind` is given.
    pub fn has_document(&self, kind: &str) -> bool {
        self.documents
            .iter()
            .any(|document| document.kind == kind && document.is_given())
    }
}

/// What `transition` still lacks, in English, or `None` when the request is
/// complete. A reason is always required; the documents are the ones each
/// transition's guard names: `cancel_onboarding` none; `withdraw_notice` the
/// other party's written agreement; `report_absconding` the MoHRE absence
/// report number and date; `reinstate` the withdrawal or cancellation
/// reference and the day the worker returned (not in the future);
/// `record_death` the death certificate or the authority's notice and the
/// date of death (required, not in the future: it fixes the salary owed up to
/// it and the date of the event).
pub fn missing(transition: &str, input: &ExitInput) -> Option<String> {
    if input.reason.trim().is_empty() {
        return Some(english("wl_hr.exits.reason_required", &[]));
    }
    let key = match transition {
        WITHDRAW_NOTICE if !input.has_document(KIND_AGREEMENT) => "agreement_required",
        REPORT_ABSCONDING if blank(&input.report_number) || input.report_date.is_none() => {
            "report_required"
        }
        REINSTATE if blank(&input.withdrawal_reference) => "reference_required",
        REINSTATE if input.return_date.is_none() => "return_date_required",
        REINSTATE if input.return_date.is_some_and(|day| day > today()) => "return_date_future",
        RECORD_DEATH if input.death_date.is_none() => "death_date_required",
        RECORD_DEATH if input.death_date.is_some_and(|day| day > today()) => "death_date_future",
        RECORD_DEATH
            if !(input.has_document(KIND_DEATH_CERTIFICATE)
                || input.has_document(KIND_AUTHORITY_NOTICE)) =>
        {
            "death_document_required"
        }
        _ => return None,
    };
    Some(english(&format!("wl_hr.exits.{key}"), &[]))
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitRefused {
    /// The name is not one of the five.
    UnknownAction,
    /// The employee is not in a state the action leaves.
    WrongState,
    /// The reason or a document is missing; the text says which.
    Incomplete(String),
    /// A request for the employee is already pending.
    AlreadyPending,
    /// No approval policy is active for the action, so it would run unchecked.
    NoPolicy,
}

impl ExitRefused {
    /// The text for the refusal.
    pub fn message(&self) -> String {
        match self {
            ExitRefused::UnknownAction => english("wl_hr.exits.unknown_action", &[]),
            ExitRefused::WrongState => english("wl_hr.exits.wrong_state", &[]),
            ExitRefused::Incomplete(message) => message.clone(),
            ExitRefused::AlreadyPending => english("wl_hr.exits.already_pending", &[]),
            ExitRefused::NoPolicy => english("wl_hr.exits.no_policy", &[]),
        }
    }

    /// The refusal as a 422 error.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// The exits that can be requested for an employee in `status`, in the
/// order of [`EXIT_TRANSITIONS`].
pub fn available_actions(status: crate::employee::EmployeeStatus) -> Vec<&'static str> {
    let machine = Employee::machine();
    EXIT_TRANSITIONS
        .into_iter()
        .filter(|name| {
            machine
                .find(name)
                .is_ok_and(|t| t.from_states().iter().any(|s| *s == status.as_str()))
        })
        .collect()
}

/// The pending request about `employee` for one of the five exits, if any.
pub async fn pending_for(db: &Db, employee: &Employee) -> Result<Option<ApprovalRequest>> {
    Ok(ApprovalRequest::pending_about(db, &employee.subject())
        .await?
        .into_iter()
        .find(|request| is_exit(&request.transition)))
}

/// Whether `user` holds `employees.manage` in one of the employee's scopes
/// (globally, when it has none).
pub(crate) async fn holds_manage(state: &AppState, user: i64, employee: &Employee) -> Result<bool> {
    let scopes = assignees::scopes_of(&state.db, &employee.scopes(&state.db).await?).await?;
    for scope in &scopes {
        if users_with_permission_in(&state.db, permissions::EMPLOYEES_MANAGE, scope)
            .await?
            .iter()
            .any(|u| u.id == user)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `by` (a holder of `employees.manage`) requests `transition` for
/// `employee_id` with `input`. The employee keeps its state; an approval
/// request exists and the approvers are told. The request is audited.
///
/// # Errors
///
/// `Forbidden` for a user without `employees.manage`; the inner
/// [`ExitRefused`] for a request that is incomplete, for a state the action
/// does not leave, or while another request for the employee is pending.
pub async fn request(
    state: &AppState,
    by: &AuthUser,
    employee_id: i64,
    transition: &str,
    input: ExitInput,
) -> Result<std::result::Result<ApprovalRequest, ExitRefused>> {
    if !is_exit(transition) {
        return Ok(Err(ExitRefused::UnknownAction));
    }
    let employee = Employee::find_or_404(&state.db, employee_id).await?;
    if !holds_manage(state, by.id, &employee).await? {
        return Err(Error::Forbidden);
    }
    if !available_actions(employee.status).contains(&transition) {
        return Ok(Err(ExitRefused::WrongState));
    }
    if let Some(message) = missing(transition, &input) {
        return Ok(Err(ExitRefused::Incomplete(message)));
    }
    if pending_for(&state.db, &employee).await?.is_some() {
        return Ok(Err(ExitRefused::AlreadyPending));
    }
    let mut input = input;
    input.has_attendance = None;
    if transition == CANCEL_ONBOARDING {
        let attended = attendance_history(state)
            .attended_days(state, employee.employee_id(), employee.hire_date, today())
            .await?;
        input.has_attendance = Some(!attended.is_empty());
    }
    if exit_effects::has_checks(transition) {
        let mut tx = state.db.begin().await?;
        let problem = exit_effects::problem_in(&mut tx, employee.id, transition, &input).await?;
        drop(tx);
        if let Some(message) = problem {
            return Ok(Err(ExitRefused::Incomplete(message)));
        }
    }
    if transition == REPORT_ABSCONDING {
        let days = *input
            .absence_days
            .get_or_insert(config_of(state).absence_days);
        let mut tx = state.db.begin().await?;
        let problem = absconding::problem_in(&mut tx, employee.id, today(), days).await?;
        drop(tx);
        if let Some(message) = problem {
            return Ok(Err(ExitRefused::Incomplete(message)));
        }
    }
    let payload = renox::serde_json::to_value(&input).map_err(|e| Error::Internal(e.into()))?;
    if ApprovalPolicy::for_action(&state.db, &action_key(transition), &payload)
        .await?
        .is_none()
    {
        return Ok(Err(ExitRefused::NoPolicy));
    }
    let outcome = employee
        .transition(transition)
        .by(by)
        .reason(input.reason.trim())
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    approvals::announce(state, &outcome).await?;
    let Some(created) = pending_for(&state.db, &employee).await? else {
        // The transition ran without a request: no policy matched after all.
        return Ok(Err(ExitRefused::NoPolicy));
    };
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(AUDIT_REQUESTED)
            .user(by.id)
            .subject(Employee::TABLE, employee.id)
            .data(audit_data(&created)),
    )
    .await?;
    Ok(Ok(created))
}

/// What the audit entries of a request carry: the transition, who asked, the
/// reason and the documents of the payload.
fn audit_data(request: &ApprovalRequest) -> Value {
    let payload = &request.payload.0;
    json!({
        "transition": request.transition,
        "approval_request": request.id,
        "requested_by": request.requester_id,
        "reason": payload["reason"],
        "documents": payload["documents"],
        "report_number": payload["report_number"],
        "report_date": payload["report_date"],
        "report_file": payload["report_file"],
        "death_date": payload["death_date"],
        "return_date": payload["return_date"],
        "withdrawal_reference": payload["withdrawal_reference"],
    })
}

/// Writes the audit entries of a request that ended: an approval gives the
/// approval and the transition; any other end gives one entry with the
/// last comment.
async fn audit_decided(state: &AppState, event: &ApprovalDecided) -> Result {
    let Some(request) = ApprovalRequest::find(&state.db, event.request).await? else {
        return Ok(());
    };
    if !is_exit(&request.transition) {
        return Ok(());
    }
    let employee = request.subject_id;
    let decider = event.approvers.last().copied();
    let mut data = audit_data(&request);
    data["approvers"] = json!(event.approvers);
    if event.decision == ApprovalEnd::Approved {
        for action in [AUDIT_APPROVED, AUDIT_TRANSITION] {
            let mut entry = renox::audit::Entry::new(action)
                .subject(Employee::TABLE, employee)
                .data(data.clone());
            if let Some(user) = decider {
                entry = entry.user(user);
            }
            renox::audit::record(&state.db, entry).await?;
        }
        return Ok(());
    }
    if let Some(decision) = request.decisions(&state.db).await?.last() {
        data["comment"] = json!(decision.comment);
    }
    let mut entry = renox::audit::Entry::new(&format!("hr.exit_{}", event.decision.as_str()))
        .subject(Employee::TABLE, employee)
        .data(data);
    if let Some(user) = decider {
        entry = entry.user(user);
    }
    renox::audit::record(&state.db, entry).await?;
    Ok(())
}

/// The checks the database answers, run again at the final approval inside
/// its transaction (the kernel's final-check hook): a message ends the
/// request `failed` and nothing runs. Today only `report_absconding` has
/// any: an absence case is open, the days passed and the contact attempts.
async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    let input: ExitInput =
        renox::serde_json::from_value(request.payload.0.clone()).unwrap_or_default();
    if exit_effects::has_checks(&request.transition) {
        return exit_effects::problem_in(tx, request.subject_id, &request.transition, &input).await;
    }
    if request.transition != REPORT_ABSCONDING {
        return Ok(None);
    }
    let days = input
        .absence_days
        .unwrap_or(absconding::DEFAULT_ABSENCE_DAYS);
    absconding::problem_in(tx, request.subject_id, today(), days).await
}

/// Registers the default policies, the final re-check and the audit listener.
pub(crate) fn register(app: &mut Registry) {
    for policy in policies() {
        policy.register();
    }
    approvals::register_final_check(EMPLOYEE_SUBJECT_TYPE, |tx, request| {
        Box::pin(recheck(tx, request))
    });
    app.listen(|event: ApprovalDecided, state: AppState| async move {
        if event.subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
            return Ok(());
        }
        audit_decided(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(reason: &str) -> ExitInput {
        ExitInput {
            reason: reason.to_owned(),
            ..ExitInput::default()
        }
    }

    fn document(kind: &str, reference: &str) -> ExitDocument {
        ExitDocument {
            kind: kind.to_owned(),
            document: None,
            reference: Some(reference.to_owned()),
        }
    }

    #[test]
    fn every_action_has_a_policy_held_by_the_exit_approvers() {
        let keys: Vec<String> = policies().iter().map(|p| p.action().to_owned()).collect();
        assert_eq!(
            keys,
            [
                "hr.cancel_onboarding",
                "hr.withdraw_notice",
                "hr.report_absconding",
                "hr.reinstate",
                "hr.record_death"
            ]
        );
        let expected =
            PolicyDefault::new("hr.report_absconding").step_permission("employees.exit.approve");
        assert_eq!(policies()[2], expected);
    }

    #[test]
    fn a_reason_is_always_required() {
        for name in EXIT_TRANSITIONS {
            let message = missing(name, &input("  ")).expect(name);
            assert_eq!(message, "Give the reason.");
        }
        assert_eq!(missing(CANCEL_ONBOARDING, &input("visa refused")), None);
    }

    #[test]
    fn each_exit_names_its_documents() {
        let mut withdraw = input("agreed");
        assert!(missing(WITHDRAW_NOTICE, &withdraw).is_some());
        withdraw.documents.push(document(KIND_AGREEMENT, "A-1"));
        assert_eq!(missing(WITHDRAW_NOTICE, &withdraw), None);

        let mut absconding = input("absent since 2026-11-01");
        assert!(missing(REPORT_ABSCONDING, &absconding).is_some());
        absconding.report_number = Some("MOHRE-777".into());
        assert!(missing(REPORT_ABSCONDING, &absconding).is_some());
        absconding.report_date = NaiveDate::from_ymd_opt(2026, 11, 9);
        assert_eq!(missing(REPORT_ABSCONDING, &absconding), None);

        let mut reinstate = input("returned");
        assert!(missing(REINSTATE, &reinstate).is_some());
        reinstate.withdrawal_reference = Some("  ".into());
        assert!(missing(REINSTATE, &reinstate).is_some());
        reinstate.withdrawal_reference = Some("WD-55".into());
        assert!(
            missing(REINSTATE, &reinstate).is_some(),
            "the day the worker returned is needed too"
        );
        reinstate.return_date = NaiveDate::from_ymd_opt(2999, 1, 1);
        assert!(
            missing(REINSTATE, &reinstate).is_some(),
            "not in the future"
        );
        reinstate.return_date = NaiveDate::from_ymd_opt(2026, 1, 10);
        assert_eq!(missing(REINSTATE, &reinstate), None);

        let mut death = input("died");
        death.death_date = NaiveDate::from_ymd_opt(2026, 1, 10);
        assert!(missing(RECORD_DEATH, &death).is_some());
        death.documents.push(document(KIND_AUTHORITY_NOTICE, ""));
        assert!(
            missing(RECORD_DEATH, &death).is_some(),
            "a blank reference is no document"
        );
        death.documents[0].document = Some(9);
        assert_eq!(missing(RECORD_DEATH, &death), None);
        death.death_date = None;
        assert_eq!(
            missing(RECORD_DEATH, &death),
            Some(english("wl_hr.exits.death_date_required", &[])),
            "the date of death is required"
        );
        death.death_date = NaiveDate::from_ymd_opt(2999, 1, 1);
        assert!(missing(RECORD_DEATH, &death).is_some(), "not in the future");
    }

    #[test]
    fn the_payload_round_trips_and_ignores_unknown_fields() {
        let mut absconding = input("absent since 2026-11-01");
        absconding.report_number = Some("MOHRE-777".into());
        absconding.report_date = NaiveDate::from_ymd_opt(2026, 11, 9);
        absconding.report_file = Some(12);
        let json = renox::serde_json::to_value(&absconding).unwrap();
        assert_eq!(json["report_number"], "MOHRE-777");
        let mut extra = json.clone();
        extra["something_else"] = json!(1);
        assert_eq!(
            renox::serde_json::from_value::<ExitInput>(extra).unwrap(),
            absconding
        );
    }

    #[test]
    fn a_refusal_is_a_422_with_its_text() {
        let refused = ExitRefused::Incomplete("Give the reason.".into());
        assert!(matches!(
            refused.into_error(),
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, text) if text == "Give the reason."
        ));
        assert_eq!(
            ExitRefused::AlreadyPending.message(),
            "A request for this employee is already waiting for approval."
        );
    }
}
