//! Counting a cash handover (wl-payments §4 *Cash handover* `count`,
//! `recount`, §5.5 steps 1-2, `FIN.ACL.05`, issue #110).
//!
//! Another user counts what the holder hands over at the end of the period
//! (`cash.count`); the holder never counts their own cash, and the person who
//! counted first never recounts. The spec's `count` and `recount` have two
//! target states each, so the machine has one transition per target:
//!
//! | Transition | From | To | When |
//! |---|---|---|---|
//! | `count` | open | counted | counted at least the expected amount |
//! | `count_short` | open | short | counted less |
//! | `recount` | short | counted | the recount reaches the expected amount |
//! | `recount_short` | short | short | the recount is still short (the difference is updated) |
//!
//! [`count`] and [`recount`] choose the transition by the amounts. A recount
//! records whether the holder was present or notified, or that the count was
//! made without the holder ("holder absent", `FIN.RUL.22`). An overage is
//! kept as a [`CashPosting`] held as a liability, not as income, until the
//! accountant decides (§12 question 4). After the commit `CashHandoverCounted`
//! is emitted, and for a short count `CashShortage` and the notice to the
//! holder and the manager ([`after_count`]).

use renox::prelude::*;
use renox::state_machine::{
    StateMachine, Transition, TransitionCtx, TransitionError, Transitioned,
};
use serde::{Deserialize, Serialize};
use wl_kernel::custody::{CashHandoverCounted, CashShortage, HandoverStatus};

use crate::cash_handover::{CashCount, CashHandover, CashPosting, CountKind, PostingKind};
use crate::cash_notices;
use crate::permissions;

/// `count`: open → counted.
pub const COUNT: &str = "count";
/// `count` with less than expected: open → short.
pub const COUNT_SHORT: &str = "count_short";
/// `recount`: short → counted.
pub const RECOUNT: &str = "recount";
/// `recount` that is still short: short → short.
pub const RECOUNT_SHORT: &str = "recount_short";

/// What a count carries to its guards and effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CountInput {
    /// What was counted, in fils.
    pub counted: i64,
    /// At a recount: whether the holder was present (or notified).
    pub holder_present: Option<bool>,
    /// The count was made without the holder (`FIN.RUL.22`).
    pub holder_absent: bool,
    /// A note.
    pub note: Option<String>,
}

fn count_input(ctx: &TransitionCtx) -> Option<CountInput> {
    ctx.input_as::<CountInput>().ok()
}

/// Why `actor` cannot count (or recount) `handover` with `input`, if they
/// cannot: no prepared input, a negative amount, the holder or the
/// responsible person counting (`FIN.ACL.05`), the first counter recounting,
/// a recount that does not say whether the holder was there, or an amount
/// that does not fit the transition (`short` says which).
pub(crate) fn count_problem(
    handover: &CashHandover,
    input: Option<&CountInput>,
    actor: Option<i64>,
    recount: bool,
    short: bool,
) -> Option<String> {
    let Some(input) = input else {
        return Some("The count was not prepared by the cash service.".to_owned());
    };
    if input.counted < 0 {
        return Some("A counted amount cannot be negative.".to_owned());
    }
    if let Some(actor) = actor {
        if actor == handover.holder_user_id || actor == handover.responsible_user_id {
            return Some(
                "Cash is counted by someone other than its holder (FIN.ACL.05).".to_owned(),
            );
        }
        if recount && handover.counted_by == Some(actor) {
            return Some("The first counter cannot recount.".to_owned());
        }
    }
    if recount && input.holder_present.is_none() && !input.holder_absent {
        return Some("Say whether the holder was present or notified.".to_owned());
    }
    let is_short = input.counted < handover.expected;
    if short && !is_short {
        return Some("The count reaches the expected amount: it is not short.".to_owned());
    }
    if !short && is_short {
        return Some("The count is below the expected amount: it is short.".to_owned());
    }
    None
}

/// The difference and the overage of a count of `counted` against `expected`.
pub(crate) fn amounts(expected: i64, counted: i64) -> (i64, i64) {
    ((expected - counted).max(0), (counted - expected).max(0))
}

/// One of the four count transitions.
fn counting(
    name: &'static str,
    label: &'static str,
    from: HandoverStatus,
    to: HandoverStatus,
) -> Transition<CashHandover> {
    let recount = from == HandoverStatus::Short;
    let short = to == HandoverStatus::Short;
    let kind = if recount {
        CountKind::Recount
    } else {
        CountKind::Count
    };
    Transition::new(name)
        .label(label)
        .from([from])
        .to(to)
        .permission(permissions::CASH_COUNT)
        .guard("counter_and_amount", move |handover: &CashHandover, ctx| {
            let problem = count_problem(
                handover,
                count_input(ctx).as_ref(),
                ctx.actor().map(|user| user.id),
                recount,
                short,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(move |handover: &CashHandover, ctx, tx| {
            let id = handover.id;
            let expected = handover.expected;
            let location = handover.location.clone();
            let legal_entity_id = handover.legal_entity_id;
            let actor = ctx.actor().map(|user| user.id).unwrap_or_default();
            let input = count_input(ctx).unwrap_or_default();
            Box::pin(async move {
                let (difference, overage) = amounts(expected, input.counted);
                let mut row = CashCount {
                    handover_id: id,
                    kind,
                    counter_id: actor,
                    amount: input.counted,
                    holder_present: input.holder_present,
                    holder_absent: input.holder_absent,
                    note: input.note.clone(),
                    ..Default::default()
                };
                row.insert(&mut *tx).await?;
                let now = Some(renox::db::now());
                let by = Some(actor);
                let query = CashHandover::query().where_(CashHandover::ID.eq(id));
                if recount {
                    query
                        .update(
                            &mut *tx,
                            &[
                                ("counted", &input.counted),
                                ("difference", &difference),
                                ("overage", &overage),
                                ("recounted_by", &by),
                                ("holder_present", &input.holder_present),
                                ("holder_absent", &input.holder_absent),
                            ],
                        )
                        .await?;
                } else {
                    query
                        .update(
                            &mut *tx,
                            &[
                                ("counted", &input.counted),
                                ("difference", &difference),
                                ("overage", &overage),
                                ("counted_by", &by),
                                ("counted_at", &now),
                                ("holder_present", &input.holder_present),
                                ("holder_absent", &input.holder_absent),
                            ],
                        )
                        .await?;
                }
                if overage > 0 {
                    // Held as a liability, not income, until the accountant
                    // decides (§12 question 4).
                    let mut posting = CashPosting {
                        handover_id: id,
                        kind: PostingKind::OverageLiability,
                        amount: overage,
                        location,
                        legal_entity_id,
                        ..Default::default()
                    };
                    posting.insert(&mut *tx).await?;
                }
                Ok(())
            })
        })
}

/// The four count transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<CashHandover>> {
    use HandoverStatus::{Counted, Open, Short};
    vec![
        counting(COUNT, "Count", Open, Counted),
        counting(COUNT_SHORT, "Count (short)", Open, Short),
        counting(RECOUNT, "Recount", Short, Counted),
        counting(RECOUNT_SHORT, "Recount (still short)", Short, Short),
    ]
}

/// Counts the `open` `handover` as `user` (`cash.count`, never the holder):
/// `counted` is what was handed over in fils. Returns the handover as it is
/// now (`counted`, or `short` with a difference). `holder_absent` records a
/// count made without the holder (`FIN.RUL.22`).
///
/// # Errors
///
/// `Forbidden` without `cash.count`; `NotFrom` unless `open`; `Guard` (422)
/// for the holder or the responsible person, or a negative amount.
pub async fn count(
    state: &AppState,
    user: &AuthUser,
    handover: &CashHandover,
    counted: i64,
    holder_absent: bool,
    note: Option<&str>,
) -> std::result::Result<CashHandover, TransitionError> {
    let handover = reload(state, handover).await?;
    let name = if counted < handover.expected {
        COUNT_SHORT
    } else {
        COUNT
    };
    let input = CountInput {
        counted,
        holder_present: None,
        holder_absent,
        note: note.map(str::to_owned),
    };
    let _outcome = handover
        .transition(name)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    reload(state, &handover).await
}

/// Recounts the `short` `handover` as `user` (`cash.count`, never the holder
/// nor the first counter): `holder_present` says whether the holder was
/// present or notified, or `holder_absent` that they were not. The
/// difference is updated; at the expected amount the handover is `counted`.
///
/// # Errors
///
/// `Forbidden` without `cash.count`; `NotFrom` unless `short`; `Guard` (422)
/// for the holder, the first counter, a recount that does not say whether the
/// holder was present, or a negative amount.
pub async fn recount(
    state: &AppState,
    user: &AuthUser,
    handover: &CashHandover,
    counted: i64,
    holder_present: Option<bool>,
    holder_absent: bool,
    note: Option<&str>,
) -> std::result::Result<CashHandover, TransitionError> {
    let handover = reload(state, handover).await?;
    let name = if counted < handover.expected {
        RECOUNT_SHORT
    } else {
        RECOUNT
    };
    let input = CountInput {
        counted,
        holder_present,
        holder_absent,
        note: note.map(str::to_owned),
    };
    let _outcome = handover
        .transition(name)
        .by(user)
        .input(&input)
        .run(state)
        .await?;
    reload(state, &handover).await
}

async fn reload(
    state: &AppState,
    handover: &CashHandover,
) -> std::result::Result<CashHandover, TransitionError> {
    CashHandover::find_or_404(&state.db, handover.id)
        .await
        .map_err(TransitionError::Failed)
}

/// After the commit of a count or recount: closes the reach-the-holder task,
/// emits `CashHandoverCounted` and, when the handover is `short`,
/// `CashShortage` and the notice to the holder and the manager.
async fn after_count(state: &AppState, event: &Transitioned<CashHandover>) -> Result {
    if ![COUNT, COUNT_SHORT, RECOUNT, RECOUNT_SHORT].contains(&event.transition.as_str()) {
        return Ok(());
    }
    let handover = CashHandover::find_or_404(&state.db, event.key).await?;
    cash_notices::close_reach_ticket(state, &handover).await?;
    state
        .emit(CashHandoverCounted {
            handover: handover.handover_id(),
            holder_user: handover.holder_user_id,
            period: handover.period_subject(),
            expected: handover.expected,
            counted: handover.counted,
            status: handover.status,
            counted_by: handover
                .recounted_by
                .or(handover.counted_by)
                .unwrap_or_default(),
        })
        .await?;
    if handover.status == HandoverStatus::Short {
        state.emit(shortage_event(&handover)).await?;
        cash_notices::shortage(state, &handover).await?;
    }
    Ok(())
}

/// The `CashShortage` of a short `handover`.
pub(crate) fn shortage_event(handover: &CashHandover) -> CashShortage {
    CashShortage {
        handover: handover.handover_id(),
        holder_user: handover.holder_user_id,
        responsible_user: handover.responsible_user_id,
        period_kind: handover.period_kind,
        period: handover.period_subject(),
        till_session: None,
        difference: handover.difference,
        missing_cheques: Vec::new(),
        source: handover.source(),
    }
}

/// Registers the listener of the count transitions.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<CashHandover>, state: AppState| async move {
            after_count(&state, &event).await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover() -> CashHandover {
        CashHandover {
            id: 1,
            holder_user_id: 5,
            responsible_user_id: 5,
            expected: 230_000,
            counted_by: Some(6),
            ..Default::default()
        }
    }

    fn input(counted: i64) -> CountInput {
        CountInput {
            counted,
            holder_present: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn the_holder_cannot_count_and_the_first_counter_cannot_recount() {
        let h = handover();
        assert!(count_problem(&h, Some(&input(225_000)), Some(5), false, true).is_some());
        assert!(count_problem(&h, Some(&input(225_000)), Some(6), false, true).is_none());
        assert!(count_problem(&h, Some(&input(230_000)), Some(6), true, false).is_some());
        assert!(count_problem(&h, Some(&input(230_000)), Some(7), true, false).is_none());
    }

    #[test]
    fn a_recount_says_whether_the_holder_was_there() {
        let h = handover();
        let mut silent = input(230_000);
        silent.holder_present = None;
        assert!(count_problem(&h, Some(&silent), Some(7), true, false).is_some());
        silent.holder_absent = true;
        assert!(count_problem(&h, Some(&silent), Some(7), true, false).is_none());
    }

    #[test]
    fn the_amount_must_fit_the_transition() {
        let h = handover();
        assert!(count_problem(&h, Some(&input(230_000)), Some(7), false, true).is_some());
        assert!(count_problem(&h, Some(&input(229_999)), Some(7), false, false).is_some());
        assert!(count_problem(&h, Some(&input(232_000)), Some(7), false, false).is_none());
        assert!(count_problem(&h, Some(&input(-1)), Some(7), false, true).is_some());
        assert!(count_problem(&h, None, Some(7), false, true).is_some());
    }

    #[test]
    fn the_difference_and_the_overage_come_from_the_count() {
        assert_eq!(amounts(230_000, 225_000), (5_000, 0));
        assert_eq!(amounts(230_000, 232_000), (0, 2_000));
        assert_eq!(amounts(230_000, 230_000), (0, 0));
    }
}
