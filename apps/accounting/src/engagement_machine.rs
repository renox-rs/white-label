//! The Engagement machine (04 §7.1 *Engagement*).
//!
//! | Transition | From | To | By |
//! |---|---|---|---|
//! | [`ACTIVATE`] | onboarding | active | `engagements.manage` |
//! | [`DECLINE`] | onboarding | declined | `engagements.manage` |
//! | [`AUTO_DECLINE`] | onboarding | declined | system (CDD `reject`) |
//! | [`SUSPEND`] | active | suspended | `engagements.manage` |
//! | [`AUTO_SUSPEND`] | active | suspended | system (`PartyBlocked`, `ACC.RUL.07`) |
//! | [`RESUME`] | suspended | active | `engagements.manage`, with a note; after a block, through the MLRO's approval |
//! | [`RESUME_APPROVED`] | suspended | active | system (the MLRO approved the resume) |
//! | [`END`] | active, suspended | ended | `engagements.end` |
//! | [`AUTO_END`] | active, suspended | ended | system (the scheduled end) |
//!
//! Every reason is a neutral code of [`crate::reasons`], given with
//! `.reason(code)` and kept on the row and in the history. The services in
//! [`crate::engagements`] gather what the guards need into an
//! [`EngagementInput`]; the effects write only the engagement's own
//! columns. What reaches other modules (the compliance calendar, the
//! retainer) runs after the commit, in [`crate::events`].
//!
//! Built here: `activate` and `decline` with their effects, and the states.
//! `suspend`, `resume` and `end` record their reason and dates; after the
//! commit a suspension holds the tasks and pauses the retainer
//! ([`crate::suspension`], BL-ACC-03) and `ACC.RUL.09` sends or prepares the
//! deadline notice ([`crate::deadline_notices`], BL-ACC-04).

use renox::chrono::NaiveDate;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, PolicyDefault};
use wl_kernel::documents::today;
use wl_kernel::machines::SubjectMachine;

use crate::db::set_engagement;
use crate::engagement::{ENGAGEMENT_SUBJECT_TYPE, Engagement, EngagementStatus};
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
/// The resume the kernel runs once the MLRO approved it (after a block).
pub const RESUME_APPROVED: &str = "resume_approved";
/// The approval action of a resume after a block (`accounting.resume`).
pub const RESUME_ACTION: &str = "accounting.resume";
/// The input key the resume policy reads: 1 when the suspension came from
/// a block (the reason is *on hold*), else 0.
pub const AFTER_BLOCK_KEY: &str = "after_block";
/// The `end` transition's name.
pub const END: &str = "end";
/// The system's `end` on the date the partner scheduled (BL-ACC-06).
pub const AUTO_END: &str = "auto_end";

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
    /// The manager's resume note (`resume`).
    #[serde(default)]
    pub note: Option<String>,
    /// 1 when the suspension came from a block, so the resume needs the
    /// MLRO's approval ([`resume_policy`]); else 0.
    #[serde(default)]
    pub after_block: i64,
    /// Every item of the End checklist still open, one line each, gathered
    /// by the service before the run (`end`, `auto_end`); the guard refuses
    /// while any is left.
    #[serde(default)]
    pub end_problems: Vec<String>,
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
            .transition(resume_approved())
            .transition(end())
            .transition(auto_end())
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

/// The guards and the effect of a resume: a note is given, the client is
/// not blocked; the reason clears and the note is kept.
fn resume_rules(transition: Transition<Engagement>) -> Transition<Engagement> {
    transition
        .from([EngagementStatus::Suspended])
        .to(EngagementStatus::Active)
        .guard("note_given", |_e: &Engagement, ctx| {
            let given = input_of(ctx)
                .note
                .is_some_and(|note| !note.trim().is_empty());
            Box::pin(async move {
                if given {
                    Ok(())
                } else {
                    Err(english("accounting.problems.resume_note", &[]))
                }
            })
        })
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
        .effect(|e: &Engagement, ctx, tx| {
            let id = e.id;
            let note = input_of(ctx).note.map(|n| n.trim().to_owned());
            Box::pin(async move {
                set_engagement(
                    &mut *tx,
                    id,
                    &[
                        ("reason_code", &None::<String>),
                        ("client_messages_suppressed", &false),
                        ("resume_note", &note),
                    ],
                )
                .await
            })
        })
}

/// `resume`: suspended → active, with the manager's note, while the client
/// is not blocked. After a block (the reason is *on hold*) the kernel's
/// approval interceptor defers it to the MLRO or deputy ([`resume_policy`]);
/// on their yes the kernel runs [`RESUME_APPROVED`], and on a no it stays
/// `suspended`.
fn resume() -> Transition<Engagement> {
    resume_rules(
        Transition::new(RESUME)
            .label("Resume")
            .tone("primary")
            .permission(permissions::ENGAGEMENTS_MANAGE),
    )
}

/// `resume_approved`: the MLRO approved the resume after a block. The
/// input is the request's payload, the manager's run as it was asked.
/// Decision: the block is checked when the manager asks (the request is not
/// created while the client is blocked); a new block meanwhile suspends
/// nothing more, the engagement being still `suspended`.
fn resume_approved() -> Transition<Engagement> {
    resume_rules(Transition::new(RESUME_APPROVED).label("Resume").system())
}

/// The default approval policy of a resume: only after a block (the input's
/// [`AFTER_BLOCK_KEY`] above 0), one step by the MLRO or deputy (the
/// compliance role, or `parties.block_reason.view`), never the manager who
/// asked.
pub fn resume_policy() -> PolicyDefault {
    PolicyDefault::new(RESUME_ACTION)
        .amount_above(0)
        .input_key(AFTER_BLOCK_KEY)
        .step_or_permission([permissions::MLRO_ROLE], permissions::BLOCK_REASON_VIEW)
}

/// The guard and the effect `end` and `auto_end` share: the End checklist is
/// clear (every open return filed or cancelled, the unbilled time billed or
/// written off, a final invoice or credit issued or nothing to bill), and the
/// end date, the reason and the restriction are kept; a scheduled end is
/// cleared.
fn ending(transition: Transition<Engagement>) -> Transition<Engagement> {
    reason_in(transition, &reasons::END_REASONS)
        .guard("checklist", |_e: &Engagement, ctx| {
            let problems = input_of(ctx).end_problems;
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
                        ("end_scheduled_on", &None::<NaiveDate>),
                        ("end_scheduled_reason", &None::<String>),
                        ("end_scheduled_by", &None::<i64>),
                    ],
                )
                .await
            })
        })
}

/// `end`: active or suspended → ended, with a reason of
/// [`reasons::END_REASONS`], on a date that is today or later (a later date
/// is scheduled by the service, not run). After a CDD exit or rejection the
/// reason is always [`RELATIONSHIP_ENDED`] and restricted (`ACC.ACL.14`),
/// whatever was chosen. Refused while the End checklist has an open item
/// (`ACC.ACL.09`, BL-ACC-06).
fn end() -> Transition<Engagement> {
    let t = Transition::new(END)
        .label("End")
        .tone("danger")
        .permission(permissions::ENGAGEMENTS_END)
        .reason_required()
        .from([EngagementStatus::Active, EngagementStatus::Suspended])
        .to(EngagementStatus::Ended)
        .guard("end_date", |_e: &Engagement, ctx| {
            let past = input_of(ctx).on.is_some_and(|on| on < today());
            Box::pin(async move {
                if past {
                    Err(english("accounting.ending.problem.past_date", &[]))
                } else {
                    Ok(())
                }
            })
        });
    ending(t)
}

/// The system's `end` on the date the partner scheduled: the same guards
/// and effects as [`END`], the end date being the scheduled date even when
/// the run is later (an earlier try was refused by the checklist).
fn auto_end() -> Transition<Engagement> {
    let t = Transition::new(AUTO_END)
        .label("End")
        .system()
        .reason_required()
        .from([EngagementStatus::Active, EngagementStatus::Suspended])
        .to(EngagementStatus::Ended);
    ending(t)
}

/// Registers the machine for `rnx state:diagram` and the admin, with the
/// kernel (the approval of a resume after a block, its default policy and
/// the engagement's page for "Needs your action").
pub(crate) fn register() {
    SubjectMachine::of::<Engagement>(ENGAGEMENT_SUBJECT_TYPE)
        .action(RESUME, RESUME_ACTION)
        .register();
    renox::state_machine::register::<Engagement>();
    resume_policy().register();
    approvals::register_subject_link(ENGAGEMENT_SUBJECT_TYPE, |id| {
        format!("/accounting/engagements/{id}")
    });
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
                (
                    AUTO_DECLINE,
                    vec!["onboarding".into()],
                    Some("declined"),
                    true
                ),
                (SUSPEND, vec!["active".into()], Some("suspended"), false),
                (AUTO_SUSPEND, vec!["active".into()], Some("suspended"), true),
                (RESUME, vec!["suspended".into()], Some("active"), false),
                (
                    RESUME_APPROVED,
                    vec!["suspended".into()],
                    Some("active"),
                    true
                ),
                (
                    END,
                    vec!["active".into(), "suspended".into()],
                    Some("ended"),
                    false
                ),
                (
                    AUTO_END,
                    vec!["active".into(), "suspended".into()],
                    Some("ended"),
                    true
                ),
            ]
        );
        let activate = machine.find(ACTIVATE).unwrap();
        assert_eq!(activate.guard_names(), ["ready"]);
        assert!(machine.find(SUSPEND).unwrap().needs_reason());
        assert_eq!(
            machine.find(END).unwrap().guard_names(),
            ["end_date", "neutral_reason", "checklist"]
        );
        assert_eq!(
            machine.find(AUTO_END).unwrap().guard_names(),
            ["neutral_reason", "checklist"]
        );
        assert_eq!(
            machine.find(RESUME).unwrap().guard_names(),
            ["note_given", "client_not_blocked"]
        );
        assert_eq!(
            wl_kernel::machines::approved(RESUME),
            RESUME_APPROVED,
            "the kernel runs it on the MLRO's yes"
        );
    }
}
