//! The till session and its state machine (wl-sales §4.3, `SAL.POS.01`; base
//! for BL-FNB-15 and the till handover of BL-PAY-19).
//!
//! ```text
//! [*] open --start_count--> counting --close--> closed --review--> reviewed
//!                                               closed --reopen--> open
//! ```
//!
//! One drawer for one shift or day. Counter sales are rung up on it
//! ([`CounterSale::till_session_id`](crate::counter_sale::CounterSale)); at
//! the end the cash is counted and the session closed, which emits
//! `TillSessionClosed` (the till's cash handover in `wl-payments` is made from
//! it). A closed session is locked; `reopen` (with a reason) puts it back to
//! `open` for a correction, and a manager or the accountant marks it
//! `reviewed`.
//!
//! Every transition has a system twin (`*_system`) for a vertical that checked
//! its own permission (06: the kitchen manager submits the daily closing), a
//! seed or a job.
//!
//! Decisions: the counted totals are the cash only (card totals come from the
//! sales and are shown, not counted); the variance above the threshold needs
//! approval in the spec (`PLT.APR.01`), which is left to the cash handover the
//! close creates (recover from the cashier or write off, `FIN.ACL.06`).

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::sales::TILL_SESSION_SUBJECT_TYPE;

use crate::permissions;
use crate::texts::english;

/// `open → counting`: sales stop on the till.
pub const START_COUNT: &str = "start_count";
/// `open → counting` by the system.
pub const START_COUNT_SYSTEM: &str = "start_count_system";
/// `counting → closed`: the counted cash is entered.
pub const CLOSE: &str = "close";
/// `counting → closed` by the system.
pub const CLOSE_SYSTEM: &str = "close_system";
/// `closed → open`: reopened for a correction, with a reason.
pub const REOPEN: &str = "reopen";
/// `closed → open` by the system.
pub const REOPEN_SYSTEM: &str = "reopen_system";
/// `closed → reviewed`.
pub const REVIEW: &str = "review";
/// `closed → reviewed` by the system.
pub const REVIEW_SYSTEM: &str = "review_system";

/// Where a till session is (§4.3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TillSessionStatus {
    /// The drawer is in use; counter sales are recorded against it.
    #[default]
    Open,
    /// Sales stopped; the cash is being counted.
    Counting,
    /// Counted and submitted; locked.
    Closed,
    /// Checked by a manager or the accountant (terminal).
    Reviewed,
}

/// A till session.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_sales_till_sessions",
    state_machine,
    index(till),
    index(status),
    index(cashier_user_id)
)]
pub struct TillSession {
    /// The primary key.
    pub id: i64,
    /// The till (register), e.g. `T1`: the holder of the cash.
    pub till: String,
    /// The outlet or branch of the till.
    pub location: Option<String>,
    /// The legal entity of the cash.
    pub legal_entity_id: Option<i64>,
    /// The trading day.
    pub business_date: Option<NaiveDate>,
    /// The cashier: the person recoveries are taken from.
    pub cashier_user_id: i64,
    /// The opening float, in fils.
    pub float_amount: i64,
    /// The cash the till should hold at the close (float plus cash sales).
    pub expected_cash: i64,
    /// The cash counted at the close.
    pub counted_cash: Option<i64>,
    /// Counted less expected (negative: a shortage).
    pub variance: Option<i64>,
    /// When the session opened.
    pub opened_at: Option<DateTime>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// Who closed it (`None`: the system, for a vertical's submit).
    pub closed_by: Option<i64>,
    /// When it was reviewed.
    pub reviewed_at: Option<DateTime>,
    /// Who reviewed it.
    pub reviewed_by: Option<i64>,
    /// The reason of the last reopening.
    pub reopen_reason: Option<String>,
    /// How many times it was reopened.
    pub reopen_count: i64,
    /// Where it is; changed only by the machine.
    pub status: TillSessionStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TillSession {
    /// The session as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(TILL_SESSION_SUBJECT_TYPE, self.id)
    }

    /// The session of `till` on `date`, if one was opened.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_day(db: &Db, till: &str, date: NaiveDate) -> Result<Option<TillSession>> {
        TillSession::query()
            .where_(TillSession::TILL.eq(till))
            .where_(TillSession::BUSINESS_DATE.eq(date))
            .order_by(TillSession::ID)
            .first(db)
            .await
    }

    /// The open session of `cashier`, the one a counter sale is rung up on.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn open_of_cashier(db: &Db, cashier: i64) -> Result<Option<TillSession>> {
        TillSession::query()
            .where_(TillSession::CASHIER_USER_ID.eq(cashier))
            .where_(TillSession::STATUS.eq(TillSessionStatus::Open))
            .order_by_desc(TillSession::ID)
            .first(db)
            .await
    }
}

/// What a transition carries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TillInput {
    /// The cash counted (`close`).
    #[serde(default)]
    pub counted: Option<i64>,
    /// The cash the till should hold, as the service read it (`close`).
    #[serde(default)]
    pub expected: i64,
}

fn input_of(ctx: &TransitionCtx) -> TillInput {
    ctx.input_as::<TillInput>().unwrap_or_default()
}

/// Sets columns of the session `id` inside the transition's transaction.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let at = now();
    all.push(("updated_at", &at));
    TillSession::query()
        .where_(TillSession::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

fn start_count(name: &'static str, system: bool) -> Transition<TillSession> {
    let transition = Transition::new(name)
        .label("Start the count")
        .tone("warning")
        .from([TillSessionStatus::Open])
        .to(TillSessionStatus::Counting);
    if system {
        transition.system()
    } else {
        transition.permission(permissions::POS_CLOSE_TILL)
    }
}

fn close(name: &'static str, system: bool) -> Transition<TillSession> {
    let transition = Transition::new(name)
        .label("Close the till")
        .tone("success")
        .from([TillSessionStatus::Counting])
        .to(TillSessionStatus::Closed)
        .guard("cash_counted", |_s: &TillSession, ctx| {
            let counted = input_of(ctx).counted;
            Box::pin(async move {
                if counted.is_some_and(|c| c >= 0) {
                    Ok(())
                } else {
                    Err(english("wl_sales.till.problem.not_counted", &[]))
                }
            })
        })
        .effect(|s: &TillSession, ctx, tx| {
            let id = s.id;
            let input = input_of(ctx);
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let counted = input.counted;
                let variance = input.counted.map(|c| c - input.expected);
                let at = Some(now());
                set(
                    tx,
                    id,
                    &[
                        ("expected_cash", &input.expected),
                        ("counted_cash", &counted),
                        ("variance", &variance),
                        ("closed_at", &at),
                        ("closed_by", &by),
                    ],
                )
                .await
            })
        });
    if system {
        transition.system()
    } else {
        transition.permission(permissions::POS_CLOSE_TILL)
    }
}

fn reopen(name: &'static str, system: bool) -> Transition<TillSession> {
    let transition = Transition::new(name)
        .label("Reopen")
        .tone("warning")
        .from([TillSessionStatus::Closed])
        .to(TillSessionStatus::Open)
        .reason_required()
        .effect(|s: &TillSession, ctx, tx| {
            let id = s.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            let count = s.reopen_count + 1;
            Box::pin(async move {
                let nothing: Option<i64> = None;
                let at: Option<DateTime> = None;
                set(
                    tx,
                    id,
                    &[
                        ("reopen_reason", &reason),
                        ("reopen_count", &count),
                        ("counted_cash", &nothing),
                        ("variance", &nothing),
                        ("closed_at", &at),
                        ("closed_by", &nothing),
                    ],
                )
                .await
            })
        });
    if system {
        transition.system()
    } else {
        transition.permission(permissions::POS_REOPEN_TILL)
    }
}

fn review(name: &'static str, system: bool) -> Transition<TillSession> {
    let transition = Transition::new(name)
        .label("Mark reviewed")
        .tone("success")
        .from([TillSessionStatus::Closed])
        .to(TillSessionStatus::Reviewed)
        .effect(|s: &TillSession, ctx, tx| {
            let id = s.id;
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let at = Some(now());
                set(tx, id, &[("reviewed_at", &at), ("reviewed_by", &by)]).await
            })
        });
    if system {
        transition.system()
    } else {
        transition.permission(permissions::POS_REVIEW)
    }
}

impl StateMachine for TillSession {
    type State = TillSessionStatus;
    const COLUMN: renox::db::Col<TillSession, TillSessionStatus> = TillSession::STATUS;

    fn machine() -> Machine<Self> {
        use TillSessionStatus::{Closed, Counting, Open, Reviewed};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("info")
            .state(Counting)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .state(Reviewed)
            .tone("success")
            .terminal()
            .transition(start_count(START_COUNT, false))
            .transition(start_count(START_COUNT_SYSTEM, true))
            .transition(close(CLOSE, false))
            .transition(close(CLOSE_SYSTEM, true))
            .transition(reopen(REOPEN, false))
            .transition(reopen(REOPEN_SYSTEM, true))
            .transition(review(REVIEW, false))
            .transition(review(REVIEW_SYSTEM, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_four_states_and_a_system_twin_of_each_transition() {
        let machine = TillSession::machine();
        assert_eq!(machine.states().len(), 4);
        for name in [START_COUNT, CLOSE, REOPEN, REVIEW] {
            assert!(!machine.find(name).unwrap().is_system(), "{name}");
        }
        for name in [
            START_COUNT_SYSTEM,
            CLOSE_SYSTEM,
            REOPEN_SYSTEM,
            REVIEW_SYSTEM,
        ] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
        assert_eq!(machine.find(REOPEN).unwrap().to_state(), Some("open"));
    }
}
