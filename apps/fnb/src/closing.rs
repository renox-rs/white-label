//! The daily closing and its state machine (06 F&B §7.1 *Daily closing*,
//! `FNB.ACL.05`, `FNB.RUL.08`; BL-FNB-15). The canonical definition is the
//! till session of wl-sales; this is the vertical's view of it: one closing
//! per outlet and business date, linked to the till session of the cash count.
//!
//! ```text
//! [*] open --submit (closing.submit)--> submitted --approve (closing.approve)--> approved
//! reopened --submit--> submitted                    submitted --reopen (closing.reopen)--> reopened
//!                                                   submitted --reopen_late (system)--> reopened
//! ```
//!
//! - `submit` needs the closing checklist done, the cash counted and the
//!   business-day cut-off passed (orders up to the cut-off belong to this
//!   day), so an outlet that closes at 23:00 submits the next morning. The
//!   shortage goes to the till's cash handover (`TillSessionClosed`).
//! - `reopen` is the area manager's, with a reason; `reopen_late` is the same
//!   by the system, when a late entry into the submitted closing was approved
//!   (`PLT.ACL.10`). `approved` is terminal: a late entry into it is refused
//!   and the area manager gets a task to record it in the next day's closing.
//!
//! Renox guards get no `AppState`, so the service ([`crate::closings`]) works
//! out what a guard needs and passes it in a [`ClosingInput`]. Every `*_system`
//! twin is for a seed, a job or the late-entry handler.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::permissions;
use crate::texts::english;

/// The subject type of a daily closing.
pub const SUBJECT_TYPE: &str = "fnb.daily_closing";

/// `open | reopened → submitted`.
pub const SUBMIT: &str = "submit";
/// `submit`, run by the system (the demo company, a job).
pub const SUBMIT_SYSTEM: &str = "submit_system";
/// `submitted → approved`.
pub const APPROVE: &str = "approve";
/// `approve`, run by the system.
pub const APPROVE_SYSTEM: &str = "approve_system";
/// `submitted → reopened`, by the area manager.
pub const REOPEN: &str = "reopen";
/// `submitted → reopened`, by the system: a late entry was approved.
pub const REOPEN_LATE: &str = "reopen_late";

/// Where a daily closing is (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClosingStatus {
    /// The trading day is running.
    #[default]
    Open,
    /// Cash counted and submitted; locked.
    Submitted,
    /// Reviewed by the area manager or the accountant (terminal).
    Approved,
    /// Sent back for correction.
    Reopened,
}

/// The daily closing of an outlet and a business date.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_daily_closings",
    state_machine,
    index(outlet_id),
    index(status)
)]
pub struct DailyClosing {
    /// The primary key.
    pub id: i64,
    /// The outlet.
    #[model(references = "fnb_outlets")]
    pub outlet_id: i64,
    /// The trading day.
    pub business_date: Option<NaiveDate>,
    /// Where it is; changed only by a transition.
    pub status: ClosingStatus,
    /// The till session (wl-sales) of the cash count.
    pub till_session_id: Option<i64>,
    /// Whether the closing checklist was completed at the last submit.
    pub checklist_done: bool,
    /// The cash the till should hold at the last submit, in fils.
    pub expected_cash: i64,
    /// The cash counted at the last submit, in fils.
    pub counted_cash: Option<i64>,
    /// Counted less expected (negative: a shortage), in fils.
    pub variance: Option<i64>,
    /// The sales of the day at the last submit, with VAT, in fils.
    pub sales_total: i64,
    /// The orders of the day at the last submit.
    pub orders_count: i64,
    /// When it was last submitted.
    pub submitted_at: Option<DateTime>,
    /// Who submitted it (`None`: the system).
    pub submitted_by: Option<i64>,
    /// When it was approved.
    pub approved_at: Option<DateTime>,
    /// Who approved it.
    pub approved_by: Option<i64>,
    /// When it was last reopened.
    pub reopened_at: Option<DateTime>,
    /// Who reopened it (`None`: the system, for a late entry).
    pub reopened_by: Option<i64>,
    /// Why it was last reopened.
    pub reopen_reason: Option<String>,
    /// How many times it was reopened.
    pub reopen_count: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DailyClosing {
    /// The closing as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The closing of `outlet_id` for `date`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_day(db: &Db, outlet_id: i64, date: NaiveDate) -> Result<Option<DailyClosing>> {
        DailyClosing::query()
            .where_(DailyClosing::OUTLET_ID.eq(outlet_id))
            .where_(DailyClosing::BUSINESS_DATE.eq(date))
            .first(db)
            .await
    }

    /// Whether the closing can still be submitted.
    pub fn can_submit(&self) -> bool {
        matches!(self.status, ClosingStatus::Open | ClosingStatus::Reopened)
    }
}

/// What a transition carries to the guards and the effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosingInput {
    /// The closing checklist was completed (`submit`).
    #[serde(default)]
    pub checklist_done: bool,
    /// The cash counted (`submit`).
    #[serde(default)]
    pub counted: Option<i64>,
    /// The cash the till should hold (`submit`).
    #[serde(default)]
    pub expected: i64,
    /// Whether the business-day cut-off has passed (`submit`).
    #[serde(default)]
    pub cutoff_passed: bool,
    /// When the closing opens, in words, for the refusal (`submit`).
    #[serde(default)]
    pub opens_text: String,
    /// The sales of the day with VAT, in fils (`submit`).
    #[serde(default)]
    pub sales_total: i64,
    /// The orders of the day (`submit`).
    #[serde(default)]
    pub orders_count: i64,
}

fn input_of(ctx: &TransitionCtx) -> ClosingInput {
    ctx.input_as::<ClosingInput>().unwrap_or_default()
}

/// Sets columns of the closing `id` inside the transition's transaction.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let at = now();
    all.push(("updated_at", &at));
    DailyClosing::query()
        .where_(DailyClosing::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

fn submit(name: &'static str, system: bool) -> Transition<DailyClosing> {
    let transition = Transition::new(name)
        .label("Submit")
        .tone("success")
        .from([ClosingStatus::Open, ClosingStatus::Reopened])
        .to(ClosingStatus::Submitted)
        .guard("checklist_done", |_c: &DailyClosing, ctx| {
            let done = input_of(ctx).checklist_done;
            Box::pin(async move {
                if done {
                    Ok(())
                } else {
                    Err(english("fnb.closing.problem.checklist", &[]))
                }
            })
        })
        .guard("cash_counted", |_c: &DailyClosing, ctx| {
            let counted = input_of(ctx).counted;
            Box::pin(async move {
                if counted.is_some_and(|c| c >= 0) {
                    Ok(())
                } else {
                    Err(english("fnb.closing.problem.not_counted", &[]))
                }
            })
        })
        .guard("cutoff_passed", |_c: &DailyClosing, ctx| {
            let input = input_of(ctx);
            Box::pin(async move {
                if input.cutoff_passed {
                    Ok(())
                } else {
                    Err(english(
                        "fnb.closing.problem.cutoff",
                        &[("opens", &input.opens_text)],
                    ))
                }
            })
        })
        .effect(|c: &DailyClosing, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let variance = input.counted.map(|counted| counted - input.expected);
                let at = Some(now());
                set(
                    tx,
                    id,
                    &[
                        ("checklist_done", &true),
                        ("expected_cash", &input.expected),
                        ("counted_cash", &input.counted),
                        ("variance", &variance),
                        ("sales_total", &input.sales_total),
                        ("orders_count", &input.orders_count),
                        ("submitted_at", &at),
                        ("submitted_by", &by),
                    ],
                )
                .await
            })
        });
    if system {
        transition.system()
    } else {
        transition.permission(permissions::CLOSING_SUBMIT)
    }
}

fn approve(name: &'static str, system: bool) -> Transition<DailyClosing> {
    let transition = Transition::new(name)
        .label("Approve")
        .tone("success")
        .from([ClosingStatus::Submitted])
        .to(ClosingStatus::Approved)
        .effect(|c: &DailyClosing, ctx, tx| {
            let id = c.id;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let at = Some(now());
                set(tx, id, &[("approved_at", &at), ("approved_by", &by)]).await
            })
        });
    if system {
        transition.system()
    } else {
        transition.permission(permissions::CLOSING_APPROVE)
    }
}

fn reopen(name: &'static str, system: bool) -> Transition<DailyClosing> {
    let transition = Transition::new(name)
        .label("Reopen")
        .tone("warning")
        .from([ClosingStatus::Submitted])
        .to(ClosingStatus::Reopened)
        .reason_required()
        .effect(|c: &DailyClosing, ctx, tx| {
            let id = c.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            let by = ctx.actor().map(|user| user.id);
            let count = c.reopen_count + 1;
            Box::pin(async move {
                let at = Some(now());
                set(
                    tx,
                    id,
                    &[
                        ("reopened_at", &at),
                        ("reopened_by", &by),
                        ("reopen_reason", &reason),
                        ("reopen_count", &count),
                    ],
                )
                .await
            })
        });
    if system {
        transition.system()
    } else {
        transition.permission(permissions::CLOSING_REOPEN)
    }
}

impl StateMachine for DailyClosing {
    type State = ClosingStatus;
    const COLUMN: renox::db::Col<DailyClosing, ClosingStatus> = DailyClosing::STATUS;

    fn machine() -> Machine<Self> {
        use ClosingStatus::{Approved, Open, Reopened, Submitted};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Reopened)
            .tone("warning")
            .state(Approved)
            .tone("success")
            .terminal()
            .transition(submit(SUBMIT, false))
            .transition(submit(SUBMIT_SYSTEM, true))
            .transition(approve(APPROVE, false))
            .transition(approve(APPROVE_SYSTEM, true))
            .transition(reopen(REOPEN, false))
            .transition(reopen(REOPEN_LATE, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_four_states_and_the_three_transitions_with_their_twins() {
        let machine = DailyClosing::machine();
        assert_eq!(machine.states().len(), 4);
        for name in [SUBMIT, APPROVE, REOPEN] {
            assert!(!machine.find(name).unwrap().is_system(), "{name}");
        }
        for name in [SUBMIT_SYSTEM, APPROVE_SYSTEM, REOPEN_LATE] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
        assert_eq!(
            machine.find(SUBMIT).unwrap().guard_names(),
            ["checklist_done", "cash_counted", "cutoff_passed"]
        );
        assert_eq!(
            machine.find(SUBMIT).unwrap().from_states(),
            ["open", "reopened"]
        );
    }
}
