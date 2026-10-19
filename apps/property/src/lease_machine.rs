//! The Lease state machine (01 Property §7.1 *Lease*).
//!
//! ```text
//! [*] draft
//! draft --submit--> pending_signature --activate--> active
//! draft, pending_signature --cancel--> cancelled
//! active --flag_renewal--> renewal_due --expire--> expired
//! active, renewal_due, expired --record_notice--> notice_given
//! notice_given --withdraw_notice--> active | renewal_due | expired
//! renewal_due, expired --renew--> renewed
//! active, renewal_due, expired --amend--> amended
//! notice_given --terminate--> terminated
//! active, renewal_due, expired, notice_given --force_terminate--> terminated
//! ```
//!
//! - `withdraw_notice` has three targets in §7.1, so it is three transitions
//!   (`withdraw_notice`, `withdraw_notice_to_renewal_due`,
//!   `withdraw_notice_to_expired`), as the other machines split a transition
//!   by target.
//! - `cancel` is the user's, asked by the leasing agent or the accountant
//!   (`leases.cancel`); when money or cheques were received a property
//!   manager must approve it (`PRP.ACL.18`, [`cancel_policy`]): the kernel's
//!   interceptor defers the run as an approval request and, at the final
//!   approval, runs `cancel_approved`, which has the same guard and effect.
//!   The money effects run after the commit of either
//!   ([`crate::cancellation`]).
//! - `submit` and `activate` check the rows they need as their first effect
//!   ([`crate::lease_checks`]); `activate` also moves the unit, the previous
//!   version and the charge schedule in the same transaction (BL-PRP-05).
//! - `flag_renewal` stores the day of the rule run ([`FlagInput`]) as the day
//!   the lease entered its renewal window (BL-PRP-03); the notices follow the
//!   commit ([`crate::lease_events`]).
//! - `expire` needs the end date to have passed and extends the charge
//!   schedule for the statutory renewal period ([`crate::holding`],
//!   BL-PRP-06); the managers are told after the commit.
//! - `renew` links the two versions, cancels the old version's scheduled
//!   charges that the new one covers and skips the periods the old version
//!   already invoiced ([`crate::renew`], BL-PRP-07).
//! - `amend`, `record_notice`, `withdraw_notice*`, `terminate` and
//!   `force_terminate` are declared with their states and permissions; their
//!   effects are built by BL-PRP-08 to BL-PRP-12 and until then the pages do
//!   not offer them.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::SubjectMachine;
use wl_kernel::approvals::PolicyDefault;
use wl_kernel::documents::today;

use crate::charge::{ChargeStatus, LeaseCharge};
use crate::db::{refused, set_lease};
use crate::holding::expire_effects;
use crate::lease::{LEASE_SUBJECT_TYPE, Lease, LeaseStatus};
use crate::lease_checks::{activate_checks, apply_activation, submit_checks};
use crate::permissions;
use crate::renew::renew_effects;
use crate::texts::english;

/// `submit`: the draft goes out for signature.
pub const SUBMIT: &str = "submit";
/// `activate`: the signed lease starts.
pub const ACTIVATE: &str = "activate";
/// `flag_renewal` (system, `PRP.RUL.01`).
pub const FLAG_RENEWAL: &str = "flag_renewal";
/// `record_notice`.
pub const RECORD_NOTICE: &str = "record_notice";
/// `withdraw_notice`, back to `active`.
pub const WITHDRAW_NOTICE: &str = "withdraw_notice";
/// `withdraw_notice`, back to `renewal_due`.
pub const WITHDRAW_NOTICE_TO_RENEWAL_DUE: &str = "withdraw_notice_to_renewal_due";
/// `withdraw_notice`, back to `expired`.
pub const WITHDRAW_NOTICE_TO_EXPIRED: &str = "withdraw_notice_to_expired";
/// `renew` (system): the renewal version was activated.
pub const RENEW: &str = "renew";
/// `amend` (system): the amendment version was activated.
pub const AMEND: &str = "amend";
/// `expire` (system, `PRP.RUL.03`).
pub const EXPIRE: &str = "expire";
/// `terminate`.
pub const TERMINATE: &str = "terminate";
/// `force_terminate`.
pub const FORCE_TERMINATE: &str = "force_terminate";
/// `cancel`: the user's.
pub const CANCEL: &str = "cancel";
/// `cancel_approved`: the kernel runs it at the final approval.
pub const CANCEL_APPROVED: &str = "cancel_approved";

/// The approval policies' action key of `cancel` (`PRP.ACL.18`).
pub const CANCEL_ACTION: &str = "property.lease_cancel";

/// The transitions the pages offer so far.
pub const OFFERED: [&str; 3] = [SUBMIT, ACTIVATE, CANCEL];

/// What a `cancel` run carries to its guard and effect; it is also the
/// approval request's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelInput {
    /// Why the lease is cancelled.
    #[serde(default)]
    pub reason: String,
    /// The part of the deposit forfeited, in fils (0 for none).
    #[serde(default)]
    pub forfeit: i64,
    /// The clause of the signed reservation or contract terms that allows the
    /// forfeit.
    #[serde(default)]
    pub forfeit_clause: Option<String>,
    /// The fils of money and cheques received by now. The approval policy
    /// reads it: nothing received, no approval. A run without it (not made by
    /// [`crate::cancellation::request`]) counts as money received.
    #[serde(default)]
    pub money_received: i64,
}

/// What a `submit` run carries: the refusal [`crate::terms::refusal`] found
/// for a renewal or amendment that changes terms against the tenant
/// (`PRP.ACL.17`), which the transition raises after its own checks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitInput {
    /// Why the version cannot be submitted, when it cannot.
    #[serde(default)]
    pub terms_refusal: Option<String>,
}

/// What a `flag_renewal` run carries: the day (Asia/Dubai) the rule ran for,
/// stored as the day the lease entered its renewal window. A run without it
/// (a test, a hand run) counts today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagInput {
    /// The day of the rule run.
    #[serde(default)]
    pub day: Option<renox::chrono::NaiveDate>,
}

/// What an `expire` run carries: the day (Asia/Dubai) the rule ran for. A run
/// without it counts today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpireInput {
    /// The day of the rule run.
    #[serde(default)]
    pub day: Option<renox::chrono::NaiveDate>,
}

/// The day an `expire` run is for.
fn expire_day(ctx: &TransitionCtx) -> renox::chrono::NaiveDate {
    ctx.input_as::<ExpireInput>()
        .ok()
        .and_then(|input| input.day)
        .unwrap_or_else(today)
}

/// What a `renew` run carries: the renewal version that was activated, so
/// that the effect does not have to look for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewInput {
    /// The renewal version.
    #[serde(default)]
    pub next: Option<i64>,
}

/// The input of a cancel run: the run's input, with the history reason as the
/// reason when the input has none.
pub fn cancel_input(ctx: &TransitionCtx) -> CancelInput {
    let mut input = ctx.input_as::<CancelInput>().unwrap_or_default();
    if input.reason.trim().is_empty()
        && let Some(reason) = ctx.reason()
    {
        input.reason = reason.to_owned();
    }
    input
}

/// Why the forfeit of `input` is refused, if it is: it is entered with the
/// clause of the signed terms, and it cannot exceed the deposit.
fn forfeit_problem(lease: &Lease, input: &CancelInput) -> Option<String> {
    if input.forfeit < 0 {
        return Some(english("property.cancel.forfeit_negative", &[]));
    }
    if input.forfeit == 0 {
        return None;
    }
    if input.forfeit_clause.as_deref().is_none_or(|c| c.trim().is_empty()) {
        return Some(english("property.cancel.clause_required", &[]));
    }
    if input.forfeit > lease.deposit_amount {
        return Some(english("property.cancel.forfeit_too_high", &[]));
    }
    None
}

/// A transition a user runs, in the scope of the lease's building.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<Lease> {
    Transition::new(name)
        .label(label)
        .permission_in(permission, |lease: &Lease| lease.scope())
}

/// A transition only the system runs.
fn system(name: &str, label: &str) -> Transition<Lease> {
    Transition::new(name).label(label).system()
}

/// The guard and effect `cancel` and `cancel_approved` share.
fn cancel_rules(transition: Transition<Lease>) -> Transition<Lease> {
    transition
        .from([LeaseStatus::Draft, LeaseStatus::PendingSignature])
        .to(LeaseStatus::Cancelled)
        .reason_required()
        .guard("forfeit", |lease: &Lease, ctx| {
            let problem = forfeit_problem(lease, &cancel_input(ctx));
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|lease: &Lease, ctx, tx| {
            let id = lease.id;
            let input = cancel_input(ctx);
            Box::pin(async move { record_cancellation(&mut *tx, id, &input).await })
        })
}

/// The effect of a cancellation inside its transaction: why, when, the
/// forfeit, and the charges that were not invoiced yet are cancelled. What
/// was invoiced, received or held is the money effects' (after the commit).
async fn record_cancellation(tx: &mut Transaction, id: i64, input: &CancelInput) -> Result {
    let clause = if input.forfeit > 0 {
        input
            .forfeit_clause
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
    } else {
        None
    };
    set_lease(
        &mut *tx,
        id,
        &[
            ("cancel_reason", &Some(input.reason.trim().to_owned())),
            ("cancelled_on", &Some(today())),
            ("cancel_forfeit", &input.forfeit.max(0)),
            ("cancel_forfeit_clause", &clause),
        ],
    )
    .await?;
    LeaseCharge::query()
        .where_(LeaseCharge::LEASE_ID.eq(id))
        .where_(LeaseCharge::STATUS.is_in([ChargeStatus::Planned, ChargeStatus::Scheduled]))
        .update(
            &mut *tx,
            &[
                ("status", &ChargeStatus::Cancelled),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

impl StateMachine for Lease {
    type State = LeaseStatus;
    const COLUMN: renox::db::Col<Lease, LeaseStatus> = Lease::STATUS;

    fn machine() -> Machine<Self> {
        use LeaseStatus::{
            Active, Amended, Cancelled, Draft, Expired, NoticeGiven, PendingSignature, Renewed,
            RenewalDue, Terminated,
        };
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(PendingSignature)
            .tone("info")
            .state(Active)
            .tone("success")
            .state(RenewalDue)
            .tone("warning")
            .state(Expired)
            .tone("warning")
            .state(NoticeGiven)
            .tone("warning")
            .state(Renewed)
            .tone("success")
            .terminal()
            .state(Amended)
            .tone("neutral")
            .terminal()
            .state(Terminated)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                user(SUBMIT, "Submit for signature", permissions::LEASES_CREATE)
                    .tone("primary")
                    .from([Draft])
                    .to(PendingSignature)
                    .effect(|lease: &Lease, ctx, tx| {
                        let lease = lease.clone();
                        let refusal = ctx
                            .input_as::<SubmitInput>()
                            .ok()
                            .and_then(|input| input.terms_refusal);
                        Box::pin(async move {
                            submit_checks(&mut *tx, &lease).await?;
                            if let Some(message) = refusal {
                                return Err(refused(message));
                            }
                            set_lease(&mut *tx, lease.id, &[("submitted_on", &Some(today()))]).await
                        })
                    }),
            )
            .transition(
                user(ACTIVATE, "Activate", permissions::LEASES_ACTIVATE)
                    .tone("success")
                    .from([PendingSignature])
                    .to(Active)
                    .effect(|lease: &Lease, _ctx, tx| {
                        let lease = lease.clone();
                        Box::pin(async move {
                            activate_checks(&mut *tx, &lease).await?;
                            apply_activation(&mut *tx, &lease).await
                        })
                    }),
            )
            .transition(
                system(FLAG_RENEWAL, "Flag for renewal")
                    .from([Active])
                    .to(RenewalDue)
                    .effect(|lease: &Lease, ctx, tx| {
                        let id = lease.id;
                        let day = ctx
                            .input_as::<FlagInput>()
                            .ok()
                            .and_then(|input| input.day)
                            .unwrap_or_else(today);
                        Box::pin(async move {
                            set_lease(&mut *tx, id, &[("renewal_flagged_on", &Some(day))]).await
                        })
                    }),
            )
            .transition(
                user(RECORD_NOTICE, "Record notice", permissions::LEASES_MANAGE)
                    .from([Active, RenewalDue, Expired])
                    .to(NoticeGiven),
            )
            .transition(
                user(WITHDRAW_NOTICE, "Withdraw notice", permissions::LEASES_MANAGE)
                    .from([NoticeGiven])
                    .to(Active),
            )
            .transition(
                user(
                    WITHDRAW_NOTICE_TO_RENEWAL_DUE,
                    "Withdraw notice",
                    permissions::LEASES_MANAGE,
                )
                .from([NoticeGiven])
                .to(RenewalDue),
            )
            .transition(
                user(
                    WITHDRAW_NOTICE_TO_EXPIRED,
                    "Withdraw notice",
                    permissions::LEASES_MANAGE,
                )
                .from([NoticeGiven])
                .to(Expired),
            )
            .transition(
                system(RENEW, "Renew")
                    .from([RenewalDue, Expired])
                    .to(Renewed)
                    .effect(|lease: &Lease, ctx, tx| {
                        let lease = lease.clone();
                        let next = ctx.input_as::<RenewInput>().ok().and_then(|input| input.next);
                        Box::pin(async move { renew_effects(&mut *tx, &lease, next).await })
                    }),
            )
            .transition(
                system(AMEND, "Amend")
                    .from([Active, RenewalDue, Expired])
                    .to(Amended),
            )
            .transition(
                system(EXPIRE, "Expire")
                    .from([RenewalDue])
                    .to(Expired)
                    .guard("end_date_passed", |lease: &Lease, ctx| {
                        let passed = expire_day(ctx) > lease.end_date;
                        Box::pin(async move {
                            if passed {
                                Ok(())
                            } else {
                                Err(english("property.holding.not_over", &[]))
                            }
                        })
                    })
                    .effect(|lease: &Lease, ctx, tx| {
                        let lease = lease.clone();
                        let day = expire_day(ctx);
                        Box::pin(async move { expire_effects(&mut *tx, &lease, day).await })
                    }),
            )
            .transition(
                user(TERMINATE, "Terminate", permissions::LEASES_MANAGE)
                    .from([NoticeGiven])
                    .to(Terminated),
            )
            .transition(
                user(
                    FORCE_TERMINATE,
                    "Force terminate",
                    permissions::LEASES_FORCE_TERMINATE,
                )
                .from([Active, RenewalDue, Expired, NoticeGiven])
                .to(Terminated),
            )
            .transition(cancel_rules(
                user(CANCEL, "Cancel", permissions::LEASES_CANCEL).tone("danger"),
            ))
            .transition(cancel_rules(system(CANCEL_APPROVED, "Cancel (approved)")))
    }
}

/// The default approval policy of `cancel` (`PRP.ACL.18`): one step by a
/// property manager, never the requester, when money or cheques were
/// received (`money_received` above 0 in the input).
pub fn cancel_policy() -> PolicyDefault {
    PolicyDefault::new(CANCEL_ACTION)
        .amount_above(0)
        .input_key("money_received")
        .step([permissions::PROPERTY_MANAGER_ROLE])
}

/// The Lease in the kernel's registry: its approval action for `cancel` and
/// the building as the scope where approvers are looked for.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Lease>(LEASE_SUBJECT_TYPE)
        .action(CANCEL, CANCEL_ACTION)
        .scope(|lease: &Lease| lease.scope())
        .build()
}

/// Registers the machine with the kernel and with `rnx state:diagram`, and
/// the default policy of `cancel`.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Lease>();
    cancel_policy().register();
}

/// Whether `event` is the transition `name` of a lease.
pub(crate) fn is(event: &Transitioned<Lease>, names: &[&str]) -> bool {
    names.contains(&event.transition.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = Lease::machine();
        let table: [(&str, &[&str], &str, bool); 14] = [
            (SUBMIT, &["draft"], "pending_signature", false),
            (ACTIVATE, &["pending_signature"], "active", false),
            (FLAG_RENEWAL, &["active"], "renewal_due", true),
            (
                RECORD_NOTICE,
                &["active", "renewal_due", "expired"],
                "notice_given",
                false,
            ),
            (WITHDRAW_NOTICE, &["notice_given"], "active", false),
            (
                WITHDRAW_NOTICE_TO_RENEWAL_DUE,
                &["notice_given"],
                "renewal_due",
                false,
            ),
            (WITHDRAW_NOTICE_TO_EXPIRED, &["notice_given"], "expired", false),
            (RENEW, &["renewal_due", "expired"], "renewed", true),
            (AMEND, &["active", "renewal_due", "expired"], "amended", true),
            (EXPIRE, &["renewal_due"], "expired", true),
            (TERMINATE, &["notice_given"], "terminated", false),
            (
                FORCE_TERMINATE,
                &["active", "renewal_due", "expired", "notice_given"],
                "terminated",
                false,
            ),
            (CANCEL, &["draft", "pending_signature"], "cancelled", false),
            (
                CANCEL_APPROVED,
                &["draft", "pending_signature"],
                "cancelled",
                true,
            ),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, by_system) in table {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), by_system, "{name}");
        }
    }

    #[test]
    fn a_lease_starts_a_draft_and_four_states_are_terminal() {
        let machine = Lease::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["renewed", "amended", "terminated", "cancelled"]);
        assert!(machine.states().iter().all(|s| s.tone.is_some()));
    }

    #[test]
    fn cancel_and_its_approved_twin_share_the_guard_and_the_reason() {
        let machine = Lease::machine();
        for name in [CANCEL, CANCEL_APPROVED] {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.guard_names(), ["forfeit"], "{name}");
            assert!(transition.needs_reason(), "{name}");
        }
        assert_eq!(wl_kernel::machines::approved(CANCEL), CANCEL_APPROVED);
    }

    #[test]
    fn the_policy_asks_a_property_manager_when_money_was_received() {
        let expected = PolicyDefault::new("property.lease_cancel")
            .amount_above(0)
            .input_key("money_received")
            .step(["property_manager"]);
        assert_eq!(cancel_policy(), expected);
        assert_eq!(cancel_policy().action(), CANCEL_ACTION);
    }

    #[test]
    fn a_forfeit_needs_a_clause_and_cannot_exceed_the_deposit() {
        let lease = Lease {
            deposit_amount: 500_000,
            ..Default::default()
        };
        let input = |forfeit, clause: Option<&str>| CancelInput {
            reason: "withdrew".into(),
            forfeit,
            forfeit_clause: clause.map(str::to_owned),
            money_received: 1,
        };
        assert!(forfeit_problem(&lease, &input(0, None)).is_none());
        assert!(forfeit_problem(&lease, &input(200_000, Some("Reservation form §4"))).is_none());
        assert!(forfeit_problem(&lease, &input(200_000, None)).is_some());
        assert!(forfeit_problem(&lease, &input(200_000, Some("  "))).is_some());
        assert!(forfeit_problem(&lease, &input(600_000, Some("§4"))).is_some());
        assert!(forfeit_problem(&lease, &input(-1, Some("§4"))).is_some());
    }
}
