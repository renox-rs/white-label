//! The Engagement machine (04 §7.1 *Engagement*).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | [`ACTIVATE`] | onboarding | active | `engagements.manage` |
//! | [`DECLINE`] | onboarding | declined | `engagements.manage` |
//! | [`AUTO_DECLINE`] | onboarding | declined | system (CDD `reject`) |
//! | [`SUSPEND`] | active | suspended | `engagements.manage` |
//! | [`AUTO_SUSPEND`] | active | suspended | system (`PartyBlocked`, `ACC.RUL.07`) |
//! | [`RESUME`] | suspended | active | `engagements.manage` |
//! | [`END`] | active, suspended | ended | `engagements.end` |
//!
//! Every reason is a neutral code of [`crate::reasons`], given with
//! `.reason(code)` and kept on the row and in the history. The services in
//! [`crate::engagements`] gather what the guards need into an
//! [`EngagementInput`]; the effects write only the engagement's own
//! columns. What reaches other modules (the compliance calendar, the
//! retainer) runs after the commit, in [`crate::events`].
//!
//! Built here: `activate` and `decline` with their effects, and the states.
//! `suspend`, `resume` and `end` record their reason and dates; their holds,
//! pauses, notices and checks come with BL-ACC-03 to BL-ACC-06.

use renox::chrono::NaiveDate;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::db::set_engagement;
use crate::engagement::{Engagement, EngagementStatus};
use crate::permissions;
use crate::reasons::{self, RELATIONSHIP_ENDED};
use crate::texts::english;

/// The `activate` transition's name.
pub const ACTIVATE: &str = "activate";
/// The `decline` transition's name.
pub const DECLINE: &str = "decline";
/// The system's `decline` after the CDD file is rejected at onboarding.
pub const AUTO_DECLINE: &str = "auto_decline";
/// The `suspend` transition's name.
pub const SUSPEND: &str = "suspend";
/// The system's `suspend` (a client block, `ACC.RUL.07`).
pub const AUTO_SUSPEND: &str = "auto_suspend";
/// The `resume` transition's name.
pub const RESUME: &str = "resume";
/// The `end` transition's name.
pub const END: &str = "end";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngagementInput {
    /// Every check that fails, one line each, gathered by the service before
    /// the run (`activate`); the guard refuses while any is left.
    #[serde(default)]
    pub problems: Vec<String>,
    /// The business day of the run (Asia/Dubai); today when empty.
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The reason is restricted: an `auto_suspend` from a block with a
    /// restricted reason, or an `end` after a CDD exit or rejection. No
    /// automatic message goes to the client then (`ACC.ACL.14`).
    #[serde(default)]
    pub restricted: bool,
    /// The client is blocked (`resume`).
    #[serde(default)]
    pub client_blocked: bool,
}

impl EngagementInput {
    /// An input for the day `on`.
    pub fn on(on: NaiveDate) -> Self {
        EngagementInput {
            on: Some(on),
            ..EngagementInput::default()
        }
    }
}

/// The input of a run; the default without one (`available`).
pub(crate) fn input_of(ctx: &TransitionCtx) -> EngagementInput {
    ctx.input_as::<EngagementInput>().unwrap_or_default()
}

impl StateMachine for Engagement {
    type State = EngagementStatus;
    const COLUMN: renox::db::Col<Engagement, EngagementStatus> = Engagement::STATUS;

    fn machine() -> Machine<Self> {
        use EngagementStatus::{Active, Declined, Ended, Onboarding, Suspended};
        Machine::new()
            .initial([Onboarding])
            .state(Onboarding)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Suspended)
            .tone("warning")
            .state(Ended)
            .tone("neutral")
            .terminal()
            .state(Declined)
            .tone("neutral")
            .terminal()
            .transition(activate())
            .transition(decline())
            .transition(auto_decline())
            .transition(suspend())
            .transition(auto_suspend())
            .transition(resume())
            .transition(end())
    }
}

/// A guard that the run's reason is one of `allowed`.
fn reason_in(
    transition: Transition<Engagement>,
    allowed: &'static [&'static str],
) -> Transition<Engagement> {
    transition.guard("neutral_reason", move |_e: &Engagement, ctx| {
        let ok = reasons::is_one_of(ctx.reason(), allowed);
        Box::pin(async move {
            if ok {
                Ok(())
            } else {
                Err(english("accounting.problems.reason_unknown", &[]))
            }
        })
    })
}

/// `activate`: onboarding → active once every check passes (CDD approved,
/// letter signed, tax profile complete, and the documents, screening and
/// block checks of [`crate::activation`]).
fn activate() -> Transition<Engagement> {
    Transition::new(ACTIVATE)
        .label("Activate")
        .tone("primary")
        .permission(permissions::ENGAGEMENTS_MANAGE)
        .from([EngagementStatus::Onboarding])
        .to(EngagementStatus::Active)
        .guard("ready", |_e: &Engagement, ctx| {
            let problems = input_of(ctx).problems;
            Box::pin(async move {
                if problems.is_empty() {
                    Ok(())
                } else {
                    Err(problems.join("\n"))
                }
            })
        })
        .effect(|e: &Engagement, ctx, tx| {
            let id = e.id;
            let on = input_of(ctx).on.unwrap_or_else(today);
            let start = e.start_date.unwrap_or(on);
            Box::pin(async move {
                set_engagement(
                    &mut *tx,
                    id,
                    &[
                        ("activated_on", &Some(on)),
                        ("start_date", &Some(start)),
                        ("reason_code", &None::<String>),
                        ("client_messages_suppressed", &false),
                    ],
                )
                .await
            })
        })
}

/// The effect that keeps the run's reason code.
fn keep_reason(transition: Transition<Engagement>) -> Transition<Engagement> {
    transition.effect(|e: &Engagement, ctx, tx| {
        let id = e.id;
        let code = ctx.reason().map(|r| r.trim().to_owned());
        let restricted = input_of(ctx).restricted;
        Box::pin(async move {
            set_engagement(
                &mut *tx,
                id,
                &[
                    ("reason_code", &code),
                    ("client_messages_suppressed", &restricted),
                ],
            )
            .await
        })
    })
}

/// `decline`: onboarding → declined, with a reason of
/// [`reasons::DECLINE_REASONS`]. The neutral template to the client comes
/// with BL-ACC-02.
fn decline() -> Transition<Engagement> {
    let t = Transition::new(DECLINE)
        .label("Decline")
        .tone("danger")
        .permission(permissions::ENGAGEMENTS_MANAGE)
        .reason_required()
        .from([EngagementStatus::Onboarding])
        .to(EngagementStatus::Declined);
    keep_reason(reason_in(t, &reasons::DECLINE_REASONS))
}

/// The system's `decline` when the CDD file is rejected at onboarding (with
/// [`reasons::NOT_ACCEPTED`], restricted: the MLRO decides what the client
/// is told).
fn auto_decline() -> Transition<Engagement> {
    let t = Transition::new(AUTO_DECLINE)
        .label("Decline")
        .system()
        .reason_required()
        .from([EngagementStatus::Onboarding])
        .to(EngagementStatus::Declined);
    keep_reason(reason_in(t, &reasons::DECLINE_REASONS))
}

/// `suspend`: active → suspended, with a reason of
/// [`reasons::SUSPEND_REASONS`] (never restricted).
fn suspend() -> Transition<Engagement> {
    let t = Transition::new(SUSPEND)
        .label("Suspend")
        .tone("warning")
        .permission(permissions::ENGAGEMENTS_MANAGE)
        .reason_required()
        .from([EngagementStatus::Active])
        .to(EngagementStatus::Suspended);
    keep_reason(reason_in(t, &reasons::SUSPEND_REASONS))
}

/// The system's `suspend`: a client block (always the neutral *on hold*)
/// or `ACC.RUL.07` (*non-payment*).
fn auto_suspend() -> Transition<Engagement> {
    let t = Transition::new(AUTO_SUSPEND)
        .label("Suspend")
        .system()
        .reason_required()
        .from([EngagementStatus::Active])
        .to(EngagementStatus::Suspended);
    keep_reason(reason_in(t, &reasons::SYSTEM_SUSPEND_REASONS))
}

/// `resume`: suspended → active while the client is not blocked. After a
/// block, the MLRO's approval comes with BL-ACC-05.
fn resume() -> Transition<Engagement> {
    Transition::new(RESUME)
        .label("Resume")
        .tone("primary")
        .permission(permissions::ENGAGEMENTS_MANAGE)
        .from([EngagementStatus::Suspended])
        .to(EngagementStatus::Active)
        .guard("client_not_blocked", |_e: &Engagement, ctx| {
            let blocked = input_of(ctx).client_blocked;
            Box::pin(async move {
                if blocked {
                    Err(english("accounting.problems.client_on_hold", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .effect(|e: &Engagement, _ctx, tx| {
            let id = e.id;
            Box::pin(async move {
                set_engagement(
                    &mut *tx,
                    id,
                    &[
                        ("reason_code", &None::<String>),
                        ("client_messages_suppressed", &false),
                    ],
                )
                .await
            })
        })
}

/// `end`: active or suspended → ended, with a reason of
/// [`reasons::END_REASONS`]. After a CDD exit or rejection the reason is
/// always [`RELATIONSHIP_ENDED`] and restricted (`ACC.ACL.14`), whatever was
/// chosen.
fn end() -> Transition<Engagement> {
    let t = Transition::new(END)
        .label("End")
        .tone("danger")
        .permission(permissions::ENGAGEMENTS_END)
        .reason_required()
        .from([EngagementStatus::Active, EngagementStatus::Suspended])
        .to(EngagementStatus::Ended);
    reason_in(t, &reasons::END_REASONS).effect(|e: &Engagement, ctx, tx| {
        let id = e.id;
        let input = input_of(ctx);
        let on = input.on.unwrap_or_else(today);
        let code = if input.restricted {
            Some(RELATIONSHIP_ENDED.to_owned())
        } else {
            ctx.reason().map(|r| r.trim().to_owned())
        };
        Box::pin(async move {
            set_engagement(
                &mut *tx,
                id,
                &[
                    ("ended_on", &Some(on)),
                    ("reason_code", &code),
                    ("client_messages_suppressed", &input.restricted),
                ],
            )
            .await
        })
    })
}

/// Registers the machine for `rnx state:diagram` and the admin.
pub(crate) fn register() {
    renox::state_machine::register::<Engagement>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = Engagement::machine();
        let states: Vec<(String, Option<String>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.clone(), s.tone.clone(), s.terminal))
            .collect();
        assert_eq!(
            states,
            vec![
                ("onboarding".into(), Some("neutral".into()), false),
                ("active".into(), Some("success".into()), false),
                ("suspended".into(), Some("warning".into()), false),
                ("ended".into(), Some("neutral".into()), true),
                ("declined".into(), Some("neutral".into()), true),
            ]
        );
        let transitions: Vec<(&str, Vec<String>, Option<&str>, bool)> = machine
            .transitions()
            .iter()
            .map(|t| {
                (
                    t.name(),
                    t.from_states().to_vec(),
                    t.to_state(),
                    t.is_system(),
                )
            })
            .collect();
        assert_eq!(
            transitions,
            vec![
                (ACTIVATE, vec!["onboarding".into()], Some("active"), false),
                (DECLINE, vec!["onboarding".into()], Some("declined"), false),
                (AUTO_DECLINE, vec!["onboarding".into()], Some("declined"), true),
                (SUSPEND, vec!["active".into()], Some("suspended"), false),
                (AUTO_SUSPEND, vec!["active".into()], Some("suspended"), true),
                (RESUME, vec!["suspended".into()], Some("active"), false),
                (
                    END,
                    vec!["active".into(), "suspended".into()],
                    Some("ended"),
                    false
                ),
            ]
        );
        let activate = machine.find(ACTIVATE).unwrap();
        assert_eq!(activate.guard_names(), ["ready"]);
        assert!(machine.find(SUSPEND).unwrap().needs_reason());
        assert_eq!(
            machine.find(END).unwrap().guard_names(),
            ["neutral_reason"]
        );
    }
}
