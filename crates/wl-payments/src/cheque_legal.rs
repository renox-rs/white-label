//! Referring a bounced cheque to a legal case (wl-payments §4
//! `refer_to_legal`, §5.3 step 4, §7, §10, issue #105).
//!
//! Usually after the `FIN.RUL.18` reminders (practice, not a guard: the
//! accountant may refer a bounced cheque at any time), a user with
//! `cheques.legal` hands the open amount of a bounced *received* cheque to a
//! legal case: **civil execution** by default, or a **criminal complaint**
//! only when the bank's return reason is on the configured criminal-grounds
//! list (account closed, payment stopped by the drawer, signature or
//! alteration irregular, `⚠ open: lawyer L-21`). The platform records the
//! case; it files nothing with a court or the police.
//!
//! Guards: a received cheque; the bank's return certificate attached; the case
//! type allowed by the return reason; the open amount above the minimum for
//! legal action (§7, AED 0: any amount above 0); a case owner that exists
//! (the user named, else the user who refers).
//!
//! The effect stores the case type, owner and day. After the commit
//! ([`after_referral`]): `ChequeReferredToLegal` is emitted for the vertical
//! whose agreement the cheque was received with; the follow-up task of the bounce is closed (the
//! `FIN.RUL.18` reminders act only on `bounced` cheques, so they stop); the
//! case owner gets a task with the evidence pack ([`evidence_pack`]) and a
//! notice; the party gets the formal notice (the default template, `⚠ open:
//! lawyer L-21`: its content is pending, and the client may edit it); the
//! holders of the manager role are told.
//!
//! Decisions on what the spec leaves open: the evidence pack lists the cheque,
//! the return certificate reference, the linked agreement and documents (the
//! invoices among them) and the bounce notices sent; the cheque image is
//! attached to the task from wl-documents by the case owner. The formal notice
//! is sent even to a blocked party: it is a legal step, not an automatic
//! reminder.

use renox::auth::User;
use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::{ChequeReferredToLegal, ChequeStatus};
use wl_kernel::documents::today;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;

use crate::cheque::{CaseType, Cheque, ChequeReturn};
use crate::config_of;
use crate::followup::{FollowupKind, FollowupTask, MANAGER_ROLE};
use crate::notices;
use crate::permissions;

/// The `refer_to_legal` transition's name.
pub const REFER_TO_LEGAL: &str = "refer_to_legal";

/// Days the party is given to pay in the formal notice (§10 default).
pub const NOTICE_DAYS: i64 = 7;

/// What a user gives to refer a cheque.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReferRequest {
    /// Civil execution (the default) or a criminal complaint.
    pub case_type: CaseType,
    /// The case owner; `None`: the user who refers.
    pub case_owner: Option<i64>,
}

/// What a `refer_to_legal` run carries to its guards and effect, gathered by
/// [`refer_to_legal`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReferInput {
    /// The case type.
    pub case_type: CaseType,
    /// The case owner's user id.
    pub case_owner: i64,
    /// Whether that user exists.
    pub owner_found: bool,
    /// Whether the return reason is on the criminal-grounds list.
    pub criminal_allowed: bool,
    /// The open amount a legal case needs to be above, in fils.
    pub minimum: i64,
}

fn refer_input(ctx: &TransitionCtx) -> Option<ReferInput> {
    ctx.input_as::<ReferInput>().ok()
}

/// Why `input` cannot refer `cheque`, if it cannot.
pub(crate) fn refer_problem(cheque: &Cheque, input: Option<&ReferInput>) -> Option<String> {
    if !cheque.is_received() {
        return Some("An issued cheque is replaced, not referred to a legal case.".to_owned());
    }
    if cheque
        .return_certificate
        .as_deref()
        .is_none_or(|c| c.trim().is_empty())
    {
        return Some("The bank's return certificate must be attached.".to_owned());
    }
    let Some(input) = input else {
        return Some("A case type and a case owner are required.".to_owned());
    };
    if input.case_type == CaseType::CriminalComplaint && !input.criminal_allowed {
        return Some(format!(
            "A criminal complaint is allowed only for the return reasons on the list; \"{}\" is not on it.",
            cheque.return_reason.as_deref().unwrap_or_default()
        ));
    }
    if cheque.open_amount <= input.minimum {
        return Some(format!(
            "The open amount must be above the minimum for legal action ({} fils).",
            input.minimum
        ));
    }
    if !input.owner_found {
        return Some("The case owner was not found.".to_owned());
    }
    None
}

/// `refer_to_legal`, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Cheque>> {
    vec![
        Transition::new(REFER_TO_LEGAL)
            .label("Refer to legal")
            .tone("danger")
            .permission(permissions::CHEQUES_LEGAL)
            .from([ChequeStatus::Bounced])
            .to(ChequeStatus::LegalCase)
            .guard("may_be_referred", |cheque: &Cheque, ctx| {
                let problem = refer_problem(cheque, refer_input(ctx).as_ref());
                Box::pin(async move { problem.map_or(Ok(()), Err) })
            })
            .effect(|cheque: &Cheque, ctx, tx| {
                let id = cheque.id;
                let input = refer_input(ctx);
                Box::pin(async move {
                    let input = input.ok_or_else(|| {
                        Error::from(renox::anyhow::anyhow!("a referral without its input"))
                    })?;
                    // Not the status column, so outside the machine's guard.
                    Cheque::query()
                        .where_(Cheque::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                ("case_type", &Some(input.case_type.as_str().to_owned())),
                                ("case_owner_id", &Some(input.case_owner)),
                                ("referred_on", &Some(today())),
                                ("updated_at", &renox::db::now()),
                            ],
                        )
                        .await?;
                    Ok(())
                })
            }),
    ]
}

/// `user` refers the bounced received `cheque` to a legal case
/// ([`ReferRequest`]).
///
/// # Errors
///
/// `Forbidden` without `cheques.legal`; `NotFrom` unless `bounced`; `Guard`
/// (422) for an issued cheque, one without its return certificate, a criminal
/// complaint for a return reason not on the list, an open amount not above the
/// minimum, or a case owner that does not exist.
pub async fn refer_to_legal(
    state: &AppState,
    user: &AuthUser,
    cheque: &Cheque,
    request: &ReferRequest,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)?;
    let config = config_of(state);
    let owner = request.case_owner.unwrap_or(user.id);
    let input = ReferInput {
        case_type: request.case_type,
        case_owner: owner,
        owner_found: User::find(&state.db, owner).await?.is_some(),
        criminal_allowed: cheque
            .return_reason
            .as_deref()
            .is_some_and(|reason| config.is_criminal_ground(reason)),
        minimum: config.legal_minimum_amount,
    };
    let _outcome = cheque
        .transition(REFER_TO_LEGAL)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    Cheque::find_or_404(&state.db, cheque.id)
        .await
        .map_err(TransitionError::Failed)
}

/// The evidence pack for the case owner: the cheque, the bank's return
/// certificate, the linked agreement, invoices and other documents, and the
/// notices sent to the party. The cheque image is attached from wl-documents.
///
/// # Errors
///
/// The database's errors.
pub async fn evidence_pack(db: &Db, cheque: &Cheque) -> Result<Value> {
    let returns = cheque.returns(db).await?;
    let linked = cheque.linked(db).await?;
    let invoices: Vec<String> = linked
        .iter()
        .filter(|d| d.subject_type == INVOICE_SUBJECT_TYPE)
        .map(ToString::to_string)
        .collect();
    let notices_sent: Vec<Value> = returns
        .iter()
        .map(|r| {
            json!({
                "notice": notices::CHEQUE_BOUNCED,
                "presentation": r.presentation,
                "on": r.returned_on.format("%Y-%m-%d").to_string(),
            })
        })
        .collect();
    Ok(json!({
        "cheque": {
            "id": cheque.id,
            "number": cheque.number,
            "bank": cheque.bank,
            "date": cheque.cheque_date.format("%Y-%m-%d").to_string(),
            "amount": cheque.amount,
            "honoured": cheque.honoured_amount,
            "settled": cheque.settled_amount,
            "open": cheque.open_amount,
            "currency": cheque.currency,
            "presentations": cheque.presentations,
        },
        "return_certificate": cheque.return_certificate,
        "return_reason": cheque.return_reason,
        "returns": returns.iter().map(|r: &ChequeReturn| json!({
            "presentation": r.presentation,
            "reason": r.reason,
            "certificate": r.certificate,
            "honoured": r.honoured_amount,
            "on": r.returned_on.format("%Y-%m-%d").to_string(),
        })).collect::<Vec<_>>(),
        "agreement": cheque.agreement().map(|a| a.to_string()),
        "invoices": invoices,
        "linked": linked.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "notices_sent": notices_sent,
        "cheque_image": "attach from the documents of the cheque",
    }))
}

/// After a committed `refer_to_legal` of `cheque_id`: see the module's
/// documentation.
///
/// # Errors
///
/// The database's errors, or a notice that failed.
pub(crate) async fn after_referral(state: &AppState, cheque_id: i64) -> Result<()> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    let Some(owner) = cheque.case_owner_id else {
        return Ok(());
    };
    // The reminders and the bounce follow-up give way to the case.
    FollowupTask::close_for_cheque(
        &state.db,
        cheque.id,
        &[FollowupKind::FollowUpCheque, FollowupKind::DecideCheque],
    )
    .await?;
    let evidence = evidence_pack(&state.db, &cheque).await?;
    let task = FollowupTask::legal_case(&cheque, owner, evidence);
    if let Some(task) = FollowupTask::create_once(&state.db, task).await? {
        notices::cheque_task(state, &cheque, &task).await?;
    }
    notices::cheque_formal_notice(state, &cheque).await?;
    let managers: Vec<i64> = users_with_role_in(&state.db, MANAGER_ROLE, &Scope::global())
        .await?
        .iter()
        .map(|manager| manager.id)
        .collect();
    notices::cheque_referred(state, &cheque, owner, &managers).await?;
    // A vertical whose agreement the cheque was received with sets its own
    // legal case flag (01 Property: a flag of type arrears on the lease).
    state
        .emit(ChequeReferredToLegal {
            cheque: cheque.cheque_id(),
            party: cheque.party(),
            agreement: cheque.agreement(),
            open_amount: cheque.open_amount,
            currency: cheque.currency.clone(),
            case_owner: owner,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounced() -> Cheque {
        Cheque {
            id: 1,
            amount: 1_000_000,
            open_amount: 1_000_000,
            presentations: 1,
            return_reason: Some("insufficient funds".into()),
            return_certificate: Some("CERT-1".into()),
            status: ChequeStatus::Bounced,
            ..Default::default()
        }
    }

    fn input(case_type: CaseType, criminal_allowed: bool) -> ReferInput {
        ReferInput {
            case_type,
            case_owner: 3,
            owner_found: true,
            criminal_allowed,
            minimum: 0,
        }
    }

    #[test]
    fn refer_to_legal_leaves_bounced_only() {
        let machine = Cheque::machine();
        let found = machine.find(REFER_TO_LEGAL).expect("transition");
        assert!(found.check_from(ChequeStatus::Bounced).is_ok());
        for other in [
            ChequeStatus::LegalCase,
            ChequeStatus::Deposited,
            ChequeStatus::Replaced,
            ChequeStatus::Cleared,
        ] {
            assert!(found.check_from(other).is_err(), "{other:?}");
        }
        assert_eq!(found.to_state(), Some("legal_case"));
    }

    #[test]
    fn a_civil_execution_needs_a_certificate_an_open_amount_and_an_owner() {
        let civil = input(CaseType::CivilExecution, false);
        assert_eq!(refer_problem(&bounced(), Some(&civil)), None);
        assert!(refer_problem(&bounced(), None).is_some());
        let no_certificate = Cheque {
            return_certificate: None,
            ..bounced()
        };
        assert!(refer_problem(&no_certificate, Some(&civil)).is_some());
        let blank = Cheque {
            return_certificate: Some("  ".into()),
            ..bounced()
        };
        assert!(refer_problem(&blank, Some(&civil)).is_some());
        let nothing_open = Cheque {
            open_amount: 0,
            ..bounced()
        };
        assert!(refer_problem(&nothing_open, Some(&civil)).is_some());
        let no_owner = ReferInput {
            owner_found: false,
            ..civil.clone()
        };
        assert!(refer_problem(&bounced(), Some(&no_owner)).is_some());
        let issued = Cheque {
            direction: wl_kernel::payments::Direction::Out,
            ..bounced()
        };
        assert!(refer_problem(&issued, Some(&civil)).is_some());
    }

    #[test]
    fn the_open_amount_must_be_above_the_minimum() {
        let mut civil = input(CaseType::CivilExecution, false);
        civil.minimum = 1_000_000;
        assert!(refer_problem(&bounced(), Some(&civil)).is_some());
        civil.minimum = 999_999;
        assert_eq!(refer_problem(&bounced(), Some(&civil)), None);
    }

    #[test]
    fn a_criminal_complaint_needs_a_return_reason_on_the_list() {
        let criminal = input(CaseType::CriminalComplaint, false);
        let message = refer_problem(&bounced(), Some(&criminal)).expect("refused");
        assert!(message.contains("insufficient funds"), "{message}");
        let allowed = input(CaseType::CriminalComplaint, true);
        assert_eq!(refer_problem(&bounced(), Some(&allowed)), None);
    }

    #[test]
    fn the_default_request_is_a_civil_execution_by_the_user_who_refers() {
        let request = ReferRequest::default();
        assert_eq!(request.case_type, CaseType::CivilExecution);
        assert_eq!(request.case_owner, None);
    }

    #[renox::test]
    async fn the_guard_reads_the_run_input() {
        let machine = Cheque::machine();
        let found = machine.find(REFER_TO_LEGAL).expect("transition");
        let ctx = |input: &ReferInput| {
            TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
        };
        assert!(
            found
                .check_guards(&bounced(), &ctx(&input(CaseType::CivilExecution, false)))
                .await
                .is_ok()
        );
        assert!(matches!(
            found
                .check_guards(&bounced(), &ctx(&input(CaseType::CriminalComplaint, false)))
                .await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            found
                .check_guards(&bounced(), &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }
}
