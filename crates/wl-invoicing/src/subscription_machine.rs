//! The Subscription machine (wl-invoicing §4 *Subscription*):
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | `mark_past_due` | active | past_due | system |
//! | `settle` | past_due, suspended | active | system |
//! | `suspend` | past_due | suspended | system (`FIN.RUL.06`) |
//! | `pause` / `pause_system` | active, past_due, paused | paused | `subscriptions.manage` / system |
//! | `resume`, `resume_past_due` / `resume_system`, `resume_past_due_system` | paused | active, past_due | `subscriptions.manage` / system |
//! | `cancel` / `cancel_system` | active, past_due, paused, suspended | cancelled | `subscriptions.manage` / system (the vertical's agreement, the party, a notice that ended) |
//!
//! The spec's `resume` goes to `active` or `past_due`; a transition has one
//! target, so each state has its own transition and the service
//! ([`crate::subscriptions::resume`]) picks one. Every system run has a
//! twin for a user, the permission check being per transition.
//!
//! The machine has no app state: the services gather what the guards read
//! into [`Facts`] and pass it as the run's input. A run without it is
//! refused by the guards. The other columns (dates, counters, causes) are
//! written by the services in the transition's transaction, and the events
//! and notices follow after the commit ([`crate::subscriptions`]).

use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::subscriptions::SubscriptionStatus;

use crate::permissions;
use crate::subscription::Subscription;
use crate::texts::english;

/// A cycle invoice is overdue (and not disputed) or was reopened.
pub const MARK_PAST_DUE: &str = "mark_past_due";
/// The outstanding cycle invoices are paid.
pub const SETTLE: &str = "settle";
/// `FIN.RUL.06` after the last retry.
pub const SUSPEND: &str = "suspend";
/// A user pause.
pub const PAUSE: &str = "pause";
/// A system pause (the agreement, a party block, a party's pause).
pub const PAUSE_SYSTEM: &str = "pause_system";
/// A user resume to `active`.
pub const RESUME: &str = "resume";
/// A user resume to `past_due`.
pub const RESUME_PAST_DUE: &str = "resume_past_due";
/// A system resume to `active`.
pub const RESUME_SYSTEM: &str = "resume_system";
/// A system resume to `past_due`.
pub const RESUME_PAST_DUE_SYSTEM: &str = "resume_past_due_system";
/// A user cancel.
pub const CANCEL: &str = "cancel";
/// A system cancel (the agreement ended, the party, a notice that ended).
pub const CANCEL_SYSTEM: &str = "cancel_system";

/// What the guards read, gathered by the services.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Facts {
    /// An outstanding cycle invoice that is not disputed and not in the
    /// grace period after an upheld dispute.
    pub chased_unpaid: bool,
    /// Such an invoice whose cycle started before the pause.
    pub chased_before_pause: bool,
    /// The outstanding cycle invoices, disputed ones included.
    pub outstanding: usize,
    /// Whether every `FIN.RUL.06` retry or reminder is done.
    pub retries_done: bool,
    /// Whether the client has suspension on (§7).
    pub suspension_enabled: bool,
    /// The pause reason to record.
    pub reason: Option<String>,
    /// Whether a user pause stays within the yearly allowance (always for a
    /// system pause).
    pub allowance_ok: bool,
    /// The pause causes still open once this run's cause is removed.
    pub causes_left: usize,
    /// Whether the party is blocked now.
    pub party_blocked: bool,
    /// Whether a user or party cancel respects the notice period (always for
    /// the agreement's cancel).
    pub notice_respected: bool,
}

/// The run's [`Facts`], if it carries them.
fn facts(ctx: &TransitionCtx) -> Option<Facts> {
    ctx.input_as::<Facts>().ok()
}

/// A guard that holds when `check` says so of the run's facts, refusing with
/// the text `key` otherwise (and when the run carries none).
fn rule(
    transition: Transition<Subscription>,
    name: &str,
    key: &'static str,
    check: fn(&Facts) -> bool,
) -> Transition<Subscription> {
    transition.guard(name.to_owned(), move |_sub: &Subscription, ctx| {
        let holds = facts(ctx).is_some_and(|f| check(&f));
        Box::pin(async move {
            if holds {
                Ok(())
            } else {
                Err(english(key).to_owned())
            }
        })
    })
}

fn mark_past_due() -> Transition<Subscription> {
    let t = Transition::new(MARK_PAST_DUE)
        .label("Mark past due")
        .tone("danger")
        .system()
        .from([SubscriptionStatus::Active])
        .to(SubscriptionStatus::PastDue);
    rule(
        t,
        "cycle_invoice_unpaid",
        "wl_invoicing.subscription.problem.nothing_unpaid",
        |f| f.chased_unpaid,
    )
}

fn settle() -> Transition<Subscription> {
    let t = Transition::new(SETTLE)
        .label("Settle")
        .tone("success")
        .system()
        .from([SubscriptionStatus::PastDue, SubscriptionStatus::Suspended])
        .to(SubscriptionStatus::Active);
    rule(
        t,
        "cycle_invoices_paid",
        "wl_invoicing.subscription.problem.still_unpaid",
        |f| f.outstanding == 0,
    )
}

fn suspend() -> Transition<Subscription> {
    let t = Transition::new(SUSPEND)
        .label("Suspend")
        .tone("danger")
        .system()
        .from([SubscriptionStatus::PastDue])
        .to(SubscriptionStatus::Suspended);
    let t = rule(
        t,
        "suspension_enabled",
        "wl_invoicing.subscription.problem.suspension_off",
        |f| f.suspension_enabled,
    );
    let t = rule(
        t,
        "retries_done",
        "wl_invoicing.subscription.problem.retries_left",
        |f| f.retries_done,
    );
    rule(
        t,
        "not_only_disputed",
        "wl_invoicing.subscription.problem.only_disputed",
        |f| f.chased_unpaid,
    )
}

/// What both pause transitions share.
fn pause_rules(t: Transition<Subscription>) -> Transition<Subscription> {
    let t = t
        .label("Pause")
        .tone("neutral")
        .from([
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Paused,
        ])
        .to(SubscriptionStatus::Paused);
    let t = rule(
        t,
        "reason_given",
        "wl_invoicing.subscription.problem.no_reason",
        |f| f.reason.as_deref().is_some_and(|r| !r.trim().is_empty()),
    );
    rule(
        t,
        "allowance_left",
        "wl_invoicing.subscription.problem.allowance",
        |f| f.allowance_ok,
    )
}

/// What the four resume transitions share.
fn resume_rules(t: Transition<Subscription>, to: SubscriptionStatus) -> Transition<Subscription> {
    let t = t
        .label("Resume")
        .tone("primary")
        .from([SubscriptionStatus::Paused])
        .to(to);
    let t = rule(
        t,
        "no_cause_left",
        "wl_invoicing.subscription.problem.cause_left",
        |f| f.causes_left == 0,
    );
    rule(
        t,
        "party_not_blocked",
        "wl_invoicing.subscription.problem.party_blocked",
        |f| !f.party_blocked,
    )
}

/// What both cancel transitions share.
fn cancel_rules(t: Transition<Subscription>) -> Transition<Subscription> {
    let t = t
        .label("Cancel")
        .tone("danger")
        .from([
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::Paused,
            SubscriptionStatus::Suspended,
        ])
        .to(SubscriptionStatus::Cancelled);
    rule(
        t,
        "notice_respected",
        "wl_invoicing.subscription.problem.notice",
        |f| f.notice_respected,
    )
}

impl StateMachine for Subscription {
    type State = SubscriptionStatus;
    const COLUMN: renox::db::Col<Subscription, SubscriptionStatus> = Subscription::STATUS;

    fn machine() -> Machine<Self> {
        use SubscriptionStatus::{Active, Cancelled, PastDue, Paused, Suspended};
        Machine::new()
            .initial([Active])
            .state(Active)
            .tone("success")
            .state(PastDue)
            .tone("warning")
            .state(Paused)
            .tone("neutral")
            .state(Suspended)
            .tone("error")
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(mark_past_due())
            .transition(settle())
            .transition(suspend())
            .transition(pause_rules(
                Transition::new(PAUSE).permission(permissions::SUBSCRIPTIONS_MANAGE),
            ))
            .transition(pause_rules(Transition::new(PAUSE_SYSTEM).system()))
            .transition(resume_rules(
                Transition::new(RESUME).permission(permissions::SUBSCRIPTIONS_MANAGE),
                Active,
            ))
            .transition(resume_rules(
                Transition::new(RESUME_PAST_DUE).permission(permissions::SUBSCRIPTIONS_MANAGE),
                PastDue,
            ))
            .transition(resume_rules(
                Transition::new(RESUME_SYSTEM).system(),
                Active,
            ))
            .transition(resume_rules(
                Transition::new(RESUME_PAST_DUE_SYSTEM).system(),
                PastDue,
            ))
            .transition(cancel_rules(
                Transition::new(CANCEL).permission(permissions::SUBSCRIPTIONS_MANAGE),
            ))
            .transition(cancel_rules(Transition::new(CANCEL_SYSTEM).system()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_states_follow_the_spec() {
        let machine = Subscription::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("active", Some("success"), false),
                ("past_due", Some("warning"), false),
                ("paused", Some("neutral"), false),
                ("suspended", Some("error"), false),
                ("cancelled", Some("neutral"), true),
            ]
        );
    }

    #[test]
    fn the_transitions_follow_the_spec() {
        let machine = Subscription::machine();
        let shape = |name: &str| {
            let t = machine.find(name).expect("transition");
            (
                t.from_states().to_vec(),
                t.to_state().map(str::to_owned),
                t.is_system(),
            )
        };
        let all = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            shape(MARK_PAST_DUE),
            (all(&["active"]), Some("past_due".into()), true)
        );
        assert_eq!(
            shape(SETTLE),
            (all(&["past_due", "suspended"]), Some("active".into()), true)
        );
        assert_eq!(
            shape(SUSPEND),
            (all(&["past_due"]), Some("suspended".into()), true)
        );
        for (name, system) in [(PAUSE, false), (PAUSE_SYSTEM, true)] {
            assert_eq!(
                shape(name),
                (
                    all(&["active", "past_due", "paused"]),
                    Some("paused".into()),
                    system
                )
            );
        }
        for (name, to, system) in [
            (RESUME, "active", false),
            (RESUME_PAST_DUE, "past_due", false),
            (RESUME_SYSTEM, "active", true),
            (RESUME_PAST_DUE_SYSTEM, "past_due", true),
        ] {
            assert_eq!(shape(name), (all(&["paused"]), Some(to.into()), system));
        }
        for (name, system) in [(CANCEL, false), (CANCEL_SYSTEM, true)] {
            assert_eq!(
                shape(name),
                (
                    all(&["active", "past_due", "paused", "suspended"]),
                    Some("cancelled".into()),
                    system
                )
            );
        }
        assert_eq!(
            machine.find(SUSPEND).unwrap().guard_names(),
            ["suspension_enabled", "retries_done", "not_only_disputed"]
        );
    }

    #[renox::test]
    async fn a_run_without_facts_is_refused() {
        let machine = Subscription::machine();
        let ctx = TransitionCtx::new(None);
        let sub = Subscription {
            status: SubscriptionStatus::Active,
            ..Default::default()
        };
        let mark = machine.find(MARK_PAST_DUE).unwrap();
        assert!(mark.check_guards(&sub, &ctx).await.is_err());
        let with = TransitionCtx::new(None).with_input(
            renox::serde_json::to_value(Facts {
                chased_unpaid: true,
                ..Default::default()
            })
            .unwrap(),
        );
        assert!(mark.check_guards(&sub, &with).await.is_ok());
    }
}
