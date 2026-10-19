//! Forcing the end of a lease for an abandoned unit or an executed eviction
//! (`force_terminate`, `PRP.ACL.19`, 01 Property §3.3, §4.1 row "The tenant
//! abandons the unit", §4.3 row "The tenant abandons the unit or is evicted",
//! §7.1 *Lease* and *Unit*; BL-PRP-12).
//!
//! When the tenant leaves without notice or a court evicts him, the lease
//! cannot end through the notice, the inspection and the key return. The
//! property manager **requests** the forced termination ([`request`]) from
//! `active`, `renewal_due`, `expired` or `notice_given` with
//!
//! - a reason: `abandoned` (an abandonment inspection with photos, done
//!   without the tenant, and the legal basis for repossession: a stated text
//!   and an attachment) or `eviction_executed` (the court judgment and the
//!   execution record, two attachments);
//! - the move-out date: the date of the abandonment inspection, or the
//!   execution date of the eviction;
//! - for each cheque dated after the move-out: *keep as evidence* or *return*.
//!   A cheque not mentioned is kept while a legal case flag is open, else
//!   returned.
//!
//! The attachments are documents of the lease ([`JUDGMENT_KIND`] and the
//! others); a missing one refuses the request and is named, so no approval
//! request is created. The kernel's interceptor defers the run as an approval
//! request for a holder of `leases.force_terminate` (default: the company
//! admin), never the requester (`PRP.ACL.19`, [`policy`]); the lease keeps
//! its state and billing continues while it waits. At the final approval the
//! kernel runs `force_terminate_approved`, which has the same guard and effect.
//!
//! **In the transition's transaction** ([`force_effects`]): as `terminate`
//! with the inspection done without the tenant: the move-out is stored, the
//! unit is vacated and the recurring charges after the move-out date are
//! cancelled ([`crate::termination::terminate_effects`]).
//!
//! **After the commit** ([`after_force`]): the cheques the requester chose to
//! keep are held as evidence in the register (never on the deposit list), the
//! money effects of `terminate` run ([`crate::termination::settle_keeping`]:
//! invoices of later periods credited pro rata, the other later cheques
//! returned against one receipt, the deposit sent to settlement, the tenant's
//! portal read-only), the legal case flags of type *eviction* are closed with
//! their outcome, and `LeaseForceTerminated` is emitted. Applying the deposit
//! to arrears and damages is BL-PRP-17; the arrears that remain stay on their
//! invoices and follow `PRP.RUL.10` or a write-off, never an automatic one.
//!
//! Decided where the spec is silent:
//!
//! - The transition is the property manager's (`leases.manage`); the approval
//!   step is the permission `leases.force_terminate` or the company admin role.
//! - An eviction flag is closed with *eviction executed* for an executed
//!   eviction and with *settled* (note: the unit was abandoned) for an
//!   abandoned unit. Flags of other types stay open.
//! - What counts as a legal basis for repossession is open (`⚠ open: lawyer
//!   (L-22)`): the platform needs the text and the attachment, and does not
//!   judge them.
//! - The documents are checked when the request is made. Without
//!   `wl-documents` every document is valid (the module's rule).

use std::collections::BTreeSet;

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::cheques::{ChequeId, ChequeStatus, cheques};
use wl_kernel::documents::{Validity, guard, today};

use crate::building::{clean, refused};
use crate::cheques::of_version;
use crate::db::set_lease;
use crate::events::LeaseForceTerminated;
use crate::lease::{Lease, LeaseStatus};
use crate::lease_machine::{FORCE_ACTION, FORCE_TERMINATE};
use crate::legal_case::{self, CaseOutcome, CaseType, LegalCase};
use crate::permissions;
use crate::termination::{TerminateInput, cutoff, settle_keeping, terminate_effects};
use crate::texts::english;

/// The document kind of the court judgment of an eviction.
pub const JUDGMENT_KIND: &str = "judgment";
/// The document kind of the execution record of an eviction.
pub const EXECUTION_RECORD_KIND: &str = "execution_record";
/// The document kind of the abandonment inspection with photos.
pub const INSPECTION_KIND: &str = "abandonment_inspection";
/// The document kind of the legal basis for repossessing an abandoned unit.
pub const REPOSSESSION_BASIS_KIND: &str = "repossession_basis";

/// Why the lease is ended by force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceReason {
    /// The tenant left without notice.
    Abandoned,
    /// A court ordered the eviction and it was executed.
    EvictionExecuted,
}

impl ForceReason {
    /// The stored text.
    pub const fn as_str(self) -> &'static str {
        match self {
            ForceReason::Abandoned => "abandoned",
            ForceReason::EvictionExecuted => "eviction_executed",
        }
    }

    /// The reason stored as `text`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "abandoned" => Some(ForceReason::Abandoned),
            "eviction_executed" => Some(ForceReason::EvictionExecuted),
            _ => None,
        }
    }

    /// The document kinds the reason needs attached to the lease, in the order
    /// the refusal names them.
    pub fn required_kinds(self) -> &'static [&'static str] {
        match self {
            ForceReason::Abandoned => &[INSPECTION_KIND, REPOSSESSION_BASIS_KIND],
            ForceReason::EvictionExecuted => &[JUDGMENT_KIND, EXECUTION_RECORD_KIND],
        }
    }
}

/// What the requester chose for the cheques dated after the move-out, as
/// stored on the lease.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeChoices {
    /// The cheques to keep as evidence.
    #[serde(default)]
    pub keep: Vec<i64>,
    /// The cheques to return.
    #[serde(default)]
    pub hand_back: Vec<i64>,
}

/// What a `force_terminate` run carries to its guard and effect; it is also
/// the approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForceInput {
    /// `abandoned` or `eviction_executed`.
    #[serde(default)]
    pub reason: String,
    /// The move-out date: the abandonment inspection date or the execution
    /// date.
    #[serde(default)]
    pub move_out: Option<NaiveDate>,
    /// The legal basis for repossession, as text (an abandoned unit).
    #[serde(default)]
    pub basis: Option<String>,
    /// The abandonment inspection found defects that need work.
    #[serde(default)]
    pub defects: bool,
    /// What to do with each later cheque.
    #[serde(default)]
    pub cheques: ChequeChoices,
    /// The day the request was made; today when empty.
    #[serde(default)]
    pub day: Option<NaiveDate>,
}

/// The input of a `force_terminate` run.
pub fn force_input(ctx: &TransitionCtx) -> ForceInput {
    ctx.input_as::<ForceInput>().unwrap_or_default()
}

/// Why `lease` cannot be ended by force with `input`, if it cannot: the
/// reason is one of the two, the move-out date is given and not before the
/// lease started, and an abandoned unit comes with the legal basis as text.
pub fn force_problem(lease: &Lease, input: &ForceInput) -> Option<String> {
    let Some(reason) = ForceReason::parse(&input.reason) else {
        return Some(english("property.force.reason_required", &[]));
    };
    let Some(move_out) = input.move_out else {
        return Some(english("property.force.move_out_required", &[]));
    };
    if move_out < lease.start_date {
        return Some(english("property.terminate.move_out_early", &[]));
    }
    if reason == ForceReason::Abandoned
        && input.basis.as_deref().is_none_or(|b| b.trim().is_empty())
    {
        return Some(english("property.force.basis_required", &[]));
    }
    None
}

/// The default approval policy of `force_terminate` (`PRP.ACL.19`): every
/// request needs one step by a holder of `leases.force_terminate` (the
/// company admin), never the requester.
pub fn policy() -> PolicyDefault {
    PolicyDefault::new(FORCE_ACTION)
        .step_or_permission([wl_kernel::COMPANY_ADMIN_ROLE], permissions::LEASES_FORCE_TERMINATE)
}

/// The effect of `force_terminate` inside its transaction: the reason and the
/// choices are stored, the inspection is the one done without the tenant, and
/// the rest is as `terminate`.
pub(crate) async fn force_effects(
    tx: &mut Transaction,
    lease: &Lease,
    input: &ForceInput,
) -> Result {
    let (Some(reason), Some(move_out)) = (ForceReason::parse(&input.reason), input.move_out)
    else {
        return Err(refused(english("property.force.reason_required", &[])));
    };
    let choices = renox::serde_json::to_string(&input.cheques)
        .map_err(|error| refused(format!("force_cheques: {error}")))?;
    set_lease(
        &mut *tx,
        lease.id,
        &[
            ("force_reason", &Some(reason.as_str().to_owned())),
            ("force_basis", &clean(input.basis.clone())),
            ("force_cheques", &Some(choices)),
            ("inspection_done_on", &Some(move_out)),
            ("inspection_defects", &input.defects),
        ],
    )
    .await?;
    let mut staged = lease.clone();
    staged.inspection_defects = input.defects;
    let ended = TerminateInput {
        move_out: Some(move_out),
        rent_due_until: None,
        keep_credit: false,
        day: input.day,
    };
    terminate_effects(&mut *tx, &staged, &ended).await
}

/// What a user enters to request a forced termination.
#[derive(Debug, Clone)]
pub struct ForceRequest {
    /// Why.
    pub reason: ForceReason,
    /// The abandonment inspection date, or the execution date of the eviction.
    pub move_out: NaiveDate,
    /// The legal basis for repossession (an abandoned unit).
    pub basis: Option<String>,
    /// The inspection found defects that need work.
    pub defects: bool,
    /// The cheques to keep as evidence (ids of the cheque register).
    pub keep: Vec<i64>,
    /// The cheques to return.
    pub hand_back: Vec<i64>,
}

impl ForceRequest {
    /// A request with `reason` and the move-out date, no basis and no choice
    /// for the cheques.
    pub fn new(reason: ForceReason, move_out: NaiveDate) -> Self {
        ForceRequest {
            reason,
            move_out,
            basis: None,
            defects: false,
            keep: Vec::new(),
            hand_back: Vec::new(),
        }
    }

    /// With the legal basis for repossession.
    pub fn basis(mut self, basis: &str) -> Self {
        self.basis = Some(basis.to_owned());
        self
    }
}

/// The evidence kinds of `reason` that `lease` lacks, as the text that names
/// the first one; `None` when it has them all.
///
/// # Errors
///
/// The document guard's errors.
pub async fn missing_evidence(
    state: &AppState,
    lease: &Lease,
    reason: ForceReason,
) -> Result<Option<String>> {
    for kind in reason.required_kinds() {
        let validity = guard(state)
            .is_valid(&state.db, &lease.subject(), kind, today())
            .await?;
        if !matches!(validity, Validity::Valid { .. }) {
            return Ok(Some(english(
                &format!("property.force.missing_{kind}"),
                &[],
            )));
        }
    }
    Ok(None)
}

/// `user` requests the forced termination of lease `id`. The evidence is
/// checked first; then the run is deferred as an approval request, announced
/// to the approvers. The outcome is `Pending`.
///
/// # Errors
///
/// `Forbidden` without `leases.manage` in the lease's building, `NotFrom`
/// unless the lease is `active`, `renewal_due`, `expired` or `notice_given`,
/// `Guard` (422) naming the missing attachment or input, or for a cheque that
/// does not belong to the lease. Nothing is written then.
pub async fn request(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    force: ForceRequest,
) -> std::result::Result<Outcome<Lease>, TransitionError> {
    let lease = Lease::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(TransitionError::Forbidden);
    }
    if !LeaseStatus::RUNNING.contains(&lease.status) {
        return Err(TransitionError::NotFrom(lease.status.as_str().to_owned()));
    }
    let input = ForceInput {
        reason: force.reason.as_str().to_owned(),
        move_out: Some(force.move_out),
        basis: clean(force.basis),
        defects: force.defects,
        cheques: ChequeChoices {
            keep: force.keep,
            hand_back: force.hand_back,
        },
        day: Some(today()),
    };
    if let Some(problem) = force_problem(&lease, &input) {
        return Err(TransitionError::Guard(problem));
    }
    if let Some(missing) = missing_evidence(state, &lease, force.reason).await? {
        return Err(TransitionError::Guard(missing));
    }
    let held: BTreeSet<i64> = of_version(state, &lease)
        .await?
        .iter()
        .map(|c| c.id.0)
        .collect();
    if input
        .cheques
        .keep
        .iter()
        .chain(&input.cheques.hand_back)
        .any(|id| !held.contains(id))
    {
        return Err(TransitionError::Guard(english(
            "property.force.cheque_unknown",
            &[],
        )));
    }
    let reason = english(
        &format!("property.force.history_{}", force.reason.as_str()),
        &[],
    );
    let outcome = lease
        .transition(FORCE_TERMINATE)
        .by(user)
        .reason(reason)
        .input(&input)
        .run(state)
        .await?;
    wl_kernel::approvals::announce(state, &outcome)
        .await
        .map_err(TransitionError::Failed)?;
    Ok(outcome)
}

/// What [`finish`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forced {
    /// The cheques held as evidence.
    pub evidence: Vec<ChequeId>,
    /// The legal case flags closed.
    pub closed_cases: Vec<i64>,
    /// Whether this run did the work (`false` for a repeat).
    pub ran: bool,
}

/// The cheques of `lease` dated after the cutoff that are still `received` or
/// `due_for_deposit`.
async fn later_cheques(state: &AppState, lease: &Lease, cut: NaiveDate) -> Result<Vec<ChequeId>> {
    Ok(of_version(state, lease)
        .await?
        .iter()
        .filter(|c| {
            matches!(c.status, ChequeStatus::Received | ChequeStatus::DueForDeposit)
                && c.date > cut
        })
        .map(|c| c.id)
        .collect())
}

/// Runs what follows the commit of the forced termination of lease `id`, on
/// behalf of `by` (the requester). Idempotent: a lease whose money effects
/// ran is left alone.
///
/// # Errors
///
/// The database's errors; the contracts' errors on the evidence and the
/// legal case flags.
pub async fn finish(state: &AppState, id: i64, by: Option<i64>) -> Result<Forced> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let (Some(reason), Some(move_out)) = (
        lease.force_reason.as_deref().and_then(ForceReason::parse),
        lease.actual_move_out,
    ) else {
        return Ok(Forced::default());
    };
    if lease.status != LeaseStatus::Terminated || lease.money_settled_at.is_some() {
        return Ok(Forced::default());
    }
    let mut done = Forced {
        ran: true,
        ..Forced::default()
    };
    let choices: ChequeChoices = lease
        .force_cheques
        .as_deref()
        .and_then(|text| renox::serde_json::from_str(text).ok())
        .unwrap_or_default();

    // The cheques held as evidence: the ones chosen, and, while a legal case
    // is open, the later ones not chosen to go back.
    let cut = cutoff(move_out, lease.rent_due_until);
    let later = later_cheques(state, &lease, cut).await?;
    let case_open = legal_case::has_open(&state.db, lease.root_id).await?;
    let keep: Vec<ChequeId> = later
        .iter()
        .filter(|c| {
            choices.keep.contains(&c.0)
                || (case_open && !choices.hand_back.contains(&c.0) && !choices.keep.contains(&c.0))
        })
        .copied()
        .collect();
    if !keep.is_empty() {
        let note = english(
            "property.force.evidence_note",
            &[("lease", &lease.number_with_version())],
        );
        done.evidence = cheques(state).hold_as_evidence(state, keep, note).await?;
    }

    // As `terminate`, without the cheques held as evidence.
    let held: BTreeSet<i64> = done.evidence.iter().map(|c| c.0).collect();
    settle_keeping(state, id, by, &held).await?;

    // The legal case flags of an eviction are closed with their outcome.
    let (outcome, note) = match reason {
        ForceReason::EvictionExecuted => (
            CaseOutcome::EvictionExecuted,
            english("property.force.case_executed", &[]),
        ),
        ForceReason::Abandoned => (
            CaseOutcome::Settled,
            english("property.force.case_abandoned", &[]),
        ),
    };
    for case in LegalCase::of_root(&state.db, lease.root_id).await? {
        if case.is_open() && case.case_type == CaseType::Eviction {
            legal_case::clear_case(state, None, case.id, outcome, Some(note.clone())).await?;
            done.closed_cases.push(case.id);
        }
    }

    state
        .emit(LeaseForceTerminated {
            lease: lease.id,
            root: lease.root_id,
            unit: lease.unit_id,
            tenant: lease.tenant(),
            reason: reason.as_str().to_owned(),
            move_out,
            requested_by: by,
            evidence_cheques: done.evidence.len(),
            closed_cases: done.closed_cases.len(),
        })
        .await?;
    Ok(done)
}

/// After the commit of `force_terminate` or `force_terminate_approved`: the
/// money effects, the evidence, the legal case flags and the event.
pub(crate) async fn after_force(state: &AppState, event: &Transitioned<Lease>) -> Result {
    let by = crate::cancellation::requester(state, event).await?;
    finish(state, event.key, by).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2027, m, d).unwrap()
    }

    fn lease() -> Lease {
        Lease {
            start_date: day(1, 1),
            ..Default::default()
        }
    }

    #[test]
    fn a_reason_is_stored_in_snake_case_and_needs_its_documents() {
        assert_eq!(ForceReason::EvictionExecuted.as_str(), "eviction_executed");
        assert_eq!(
            ForceReason::parse("abandoned"),
            Some(ForceReason::Abandoned)
        );
        assert_eq!(ForceReason::parse("other"), None);
        assert_eq!(
            ForceReason::EvictionExecuted.required_kinds(),
            [JUDGMENT_KIND, EXECUTION_RECORD_KIND]
        );
        assert_eq!(
            ForceReason::Abandoned.required_kinds(),
            [INSPECTION_KIND, REPOSSESSION_BASIS_KIND]
        );
    }

    #[test]
    fn a_forced_termination_needs_a_reason_a_date_and_for_abandonment_a_basis() {
        let mut input = ForceInput::default();
        assert!(force_problem(&lease(), &input).unwrap().contains("reason"));
        input.reason = "eviction_executed".into();
        assert!(force_problem(&lease(), &input).unwrap().contains("move-out"));
        input.move_out = Some(day(7, 20));
        assert_eq!(force_problem(&lease(), &input), None);
        input.move_out = Some(day(1, 1) - renox::chrono::Duration::days(1));
        assert!(force_problem(&lease(), &input).is_some());
        input.move_out = Some(day(7, 20));
        input.reason = "abandoned".into();
        assert!(force_problem(&lease(), &input).unwrap().contains("legal basis"));
        input.basis = Some("  ".into());
        assert!(force_problem(&lease(), &input).is_some());
        input.basis = Some("Municipality order 12/2027".into());
        assert_eq!(force_problem(&lease(), &input), None);
    }

    #[test]
    fn the_choices_for_the_cheques_survive_the_json_column() {
        let choices = ChequeChoices {
            keep: vec![3, 4],
            hand_back: vec![5],
        };
        let text = renox::serde_json::to_string(&choices).unwrap();
        let back: ChequeChoices = renox::serde_json::from_str(&text).unwrap();
        assert_eq!(back, choices);
    }

    #[test]
    fn the_policy_asks_the_company_admin_or_a_holder_of_the_permission() {
        let expected = PolicyDefault::new("property.lease_force_terminate")
            .step_or_permission(["admin"], "leases.force_terminate");
        assert_eq!(policy(), expected);
    }
}
