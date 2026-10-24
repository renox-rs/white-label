//! Cheques collected in the field and confirmed in a cash handover
//! (wl-payments §4 *Cash handover* `count`, `recount`, `recover`,
//! `write_off`, the paragraph "A missing cheque is not a cash shortage", §5.3
//! step 6, `FIN.ACL.20`, issue #111).
//!
//! - **Expected cheques** ([`to_tick`]): the received cheques the holder
//!   collected in the field in the handover's period (`field_collected`, a
//!   `collected_by` that is the holder, registered on the period's day) that no
//!   handover confirmed yet. At a recount they are the cheques still `missing`.
//! - **The count ticks them** ([`ChequeTicks`]): each present or missing, none
//!   left out, none that is not expected. A present cheque gets its custody
//!   confirmed (`custody_handover_id` names the handover) and can be
//!   deposited; a missing one makes the handover `short` even when the cash is
//!   complete (the difference stays 0: a missing cheque is never cash).
//! - **A recount finds** a missing cheque again (`found`, its custody is
//!   confirmed) or keeps it missing.
//! - **A missing cheque is dealt with** when it is found, or returned with the
//!   reason "lost" ([`LOST`]): the party is asked for a replacement
//!   ([`on_returned`], notice `payments.cheque_replacement`) and the row is
//!   `returned_lost`. `recover` and `write_off` wait for that
//!   ([`unresolved`]).
//!
//! A holder who only collects cheques (08 delivery drivers and sales reps)
//! has expected cash 0: the handover is made when the field cheque is
//! registered ([`ensure_handover`]) and is `counted` when every cheque is
//! present.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::custody::{HandoverKind, HandoverStatus, PeriodKind};
use wl_kernel::documents::dubai_date;

use crate::cash_handover::{CashHandover, CashHolder};
use crate::cash_notices;
use crate::cheque::Cheque;
use crate::cheque_return::{RETURN, RETURN_BY_SYSTEM};

/// The reason that returns a cheque lost in custody.
pub const LOST: &str = "lost";

/// Where an expected cheque stands in a handover.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChequeTick {
    /// The counter ticked it present; its custody is confirmed.
    #[default]
    Present,
    /// The counter ticked it missing.
    Missing,
    /// A recount found a missing cheque; its custody is confirmed.
    Found,
    /// A missing cheque was returned with the reason "lost".
    ReturnedLost,
}

/// An expected cheque of a handover and what the count said of it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_payments_cash_cheques",
    unique(handover_id, cheque_id),
    index(handover_id)
)]
pub struct CashCheque {
    /// The primary key.
    pub id: i64,
    /// The handover.
    pub handover_id: i64,
    /// The cheque.
    pub cheque_id: i64,
    /// Present, missing, found or returned as lost.
    pub state: ChequeTick,
    /// The counter who ticked it last.
    pub ticked_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CashCheque {
    /// The rows of `handover_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_handover(db: &Db, handover_id: i64) -> Result<Vec<CashCheque>> {
        CashCheque::query()
            .where_(CashCheque::HANDOVER_ID.eq(handover_id))
            .order_by(CashCheque::ID)
            .get(db)
            .await
    }
}

/// What a count says of the expected cheques: the ids of those present and of
/// those missing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChequeTicks {
    /// The cheques the counter has in hand.
    pub present: Vec<i64>,
    /// The cheques the counter did not get.
    pub missing: Vec<i64>,
}

impl ChequeTicks {
    /// Ticks `present`, and every other expected cheque of `expected` missing.
    pub fn present_of(expected: &[i64], present: &[i64]) -> Self {
        ChequeTicks {
            present: expected
                .iter()
                .copied()
                .filter(|id| present.contains(id))
                .collect(),
            missing: expected
                .iter()
                .copied()
                .filter(|id| !present.contains(id))
                .collect(),
        }
    }
}

/// Why `ticks` do not account for the `expected` cheques, if they do not:
/// each expected cheque ticked once, present or missing, and nothing else.
pub(crate) fn ticks_problem(expected: &[i64], ticks: &ChequeTicks) -> Option<String> {
    let mut ticked: Vec<i64> = ticks.present.clone();
    ticked.extend(ticks.missing.iter().copied());
    ticked.sort_unstable();
    let before = ticked.len();
    ticked.dedup();
    if ticked.len() != before {
        return Some("A cheque is ticked both present and missing, or twice.".to_owned());
    }
    let mut wanted = expected.to_vec();
    wanted.sort_unstable();
    if ticked == wanted {
        None
    } else if ticked.iter().any(|id| !wanted.contains(id)) {
        Some("A ticked cheque is not one the holder is expected to hand over.".to_owned())
    } else {
        Some("Tick each expected cheque present or missing (FIN.ACL.20).".to_owned())
    }
}

/// The cheques `handover` expects to be ticked now: for a first count the
/// received field cheques the holder collected on the period's day that no
/// handover confirmed; for a recount the cheques still missing.
///
/// # Errors
///
/// Database errors.
pub async fn to_tick(db: &Db, handover: &CashHandover, recount: bool) -> Result<Vec<i64>> {
    if recount {
        return Ok(CashCheque::of_handover(db, handover.id)
            .await?
            .iter()
            .filter(|row| row.state == ChequeTick::Missing)
            .map(|row| row.cheque_id)
            .collect());
    }
    let mut ids = Vec::new();
    let cheques = Cheque::query()
        .where_(Cheque::COLLECTED_BY.eq(Some(handover.holder_user_id)))
        .where_(Cheque::FIELD_COLLECTED.eq(true))
        .order_by(Cheque::ID)
        .get(db)
        .await?;
    for cheque in cheques {
        let received = matches!(
            cheque.status,
            ChequeStatus::Received | ChequeStatus::DueForDeposit
        );
        let day = cheque.created_at.map(dubai_date);
        if received && cheque.custody_handover_id.is_none() && day == Some(handover.period_date) {
            ids.push(cheque.id);
        }
    }
    Ok(ids)
}

/// Writes the ticks of a count in `tx`: a present cheque's custody is
/// confirmed by `handover_id`; at a recount a cheque ticked present again was
/// found.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn apply(
    tx: &mut Transaction,
    handover_id: i64,
    ticks: &ChequeTicks,
    recount: bool,
    by: i64,
) -> Result {
    for id in &ticks.present {
        confirm(tx, *id, handover_id).await?;
        if recount {
            CashCheque::query()
                .where_(CashCheque::HANDOVER_ID.eq(handover_id))
                .where_(CashCheque::CHEQUE_ID.eq(*id))
                .update(
                    &mut *tx,
                    &[
                        ("state", &ChequeTick::Found.as_str().to_owned()),
                        ("ticked_by", &Some(by)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        } else {
            let mut row = CashCheque {
                handover_id,
                cheque_id: *id,
                state: ChequeTick::Present,
                ticked_by: Some(by),
                ..CashCheque::default()
            };
            row.insert(&mut *tx).await?;
        }
    }
    if !recount {
        for id in &ticks.missing {
            let mut row = CashCheque {
                handover_id,
                cheque_id: *id,
                state: ChequeTick::Missing,
                ticked_by: Some(by),
                ..CashCheque::default()
            };
            row.insert(&mut *tx).await?;
        }
    }
    Ok(())
}

/// Confirms the custody of cheque `cheque_id` in handover `handover_id`.
async fn confirm(tx: &mut Transaction, cheque_id: i64, handover_id: i64) -> Result {
    Cheque::query()
        .where_(Cheque::ID.eq(cheque_id))
        .update(
            &mut *tx,
            &[
                ("custody_handover_id", &Some(handover_id)),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// How many cheques of `handover_id` are missing and not dealt with yet.
///
/// # Errors
///
/// Database errors.
pub async fn unresolved(db: &Db, handover_id: i64) -> Result<i64> {
    let count = CashCheque::query()
        .where_(CashCheque::HANDOVER_ID.eq(handover_id))
        .where_(CashCheque::STATE.eq(ChequeTick::Missing))
        .count(db)
        .await?;
    Ok(i64::try_from(count).unwrap_or(i64::MAX))
}

/// Whether a cheque of `handover_id` is missing or was returned as lost: the
/// handover was short because of a cheque, not only of cash.
///
/// # Errors
///
/// Database errors.
pub async fn had_missing(db: &Db, handover_id: i64) -> Result<bool> {
    Ok(CashCheque::of_handover(db, handover_id)
        .await?
        .iter()
        .any(|row| matches!(row.state, ChequeTick::Missing | ChequeTick::ReturnedLost)))
}

/// The ids of the cheques of `handover_id` still missing, for `CashShortage`.
///
/// # Errors
///
/// Database errors.
pub async fn missing_ids(db: &Db, handover_id: i64) -> Result<Vec<i64>> {
    Ok(CashCheque::of_handover(db, handover_id)
        .await?
        .iter()
        .filter(|row| row.state == ChequeTick::Missing)
        .map(|row| row.cheque_id)
        .collect())
}

/// Returns the missing cheque `cheque_id` with the reason "lost" as `user`
/// (`cheques.manage`): the party is asked for a replacement after the commit.
///
/// # Errors
///
/// `Forbidden` without `cheques.manage`; `NotFrom` unless the cheque is
/// `received` or `due_for_deposit`.
pub async fn return_lost(
    state: &AppState,
    user: &AuthUser,
    cheque_id: i64,
) -> std::result::Result<Cheque, TransitionError> {
    let cheque = Cheque::find_or_404(&state.db, cheque_id).await?;
    let _outcome = cheque
        .transition(RETURN)
        .by(user)
        .reason(LOST)
        .run(state)
        .await?;
    Cheque::find_or_404(&state.db, cheque_id)
        .await
        .map_err(TransitionError::Failed)
}

/// After a cheque is returned with the reason "lost": the rows that expected
/// it are `returned_lost` and the party is asked for a replacement.
async fn after_returned(state: &AppState, event: &Transitioned<Cheque>) -> Result {
    if event.transition != RETURN && event.transition != RETURN_BY_SYSTEM {
        return Ok(());
    }
    let lost = event
        .reason
        .as_deref()
        .is_some_and(|reason| reason.trim().eq_ignore_ascii_case(LOST));
    if !lost {
        return Ok(());
    }
    CashCheque::query()
        .where_(CashCheque::CHEQUE_ID.eq(event.key))
        .where_(CashCheque::STATE.eq(ChequeTick::Missing))
        .update(
            &state.db,
            &[
                ("state", &ChequeTick::ReturnedLost.as_str().to_owned()),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    let cheque = Cheque::find_or_404(&state.db, event.key).await?;
    cash_notices::ask_replacement(state, &cheque).await
}

/// Makes the day's open handover of `user_id` for a cheque collected in the
/// field, when there is none: a holder who only collects cheques has expected
/// cash 0 and counts like any other.
///
/// # Errors
///
/// Database errors.
pub async fn ensure_handover(
    state: &AppState,
    user_id: i64,
    legal_entity_id: Option<i64>,
) -> Result {
    let day = dubai_date(renox::db::now());
    let holder = CashHolder::ensure(&state.db, user_id, None, legal_entity_id).await?;
    let existing = CashHandover::query()
        .where_(CashHandover::HOLDER_USER_ID.eq(user_id))
        .where_(CashHandover::KIND.eq(HandoverKind::Period))
        .where_(CashHandover::PERIOD_KIND.eq(PeriodKind::Day))
        .where_(CashHandover::PERIOD_DATE.eq(day))
        .exists(&state.db)
        .await?;
    if existing {
        return Ok(());
    }
    let mut created = CashHandover {
        kind: HandoverKind::Period,
        holder_id: holder.id,
        holder_user_id: user_id,
        responsible_user_id: user_id,
        period_kind: PeriodKind::Day,
        period_date: day,
        escalate: true,
        currency: "AED".to_owned(),
        location: holder.location.clone(),
        legal_entity_id: holder.legal_entity_id,
        status: HandoverStatus::Open,
        ..CashHandover::default()
    };
    let mut tx = state.db.begin().await?;
    created
        .insert_in_state(&mut tx, HandoverStatus::Open, "Cheque collected")
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Registers the listener of the returned cheques.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Cheque>, state: AppState| async move {
        after_returned(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_expected_cheque_is_ticked_once_and_nothing_else() {
        let expected = [101, 102];
        let both = ChequeTicks {
            present: vec![101],
            missing: vec![102],
        };
        assert_eq!(ticks_problem(&expected, &both), None);
        let left_out = ChequeTicks {
            present: vec![101],
            missing: vec![],
        };
        assert!(ticks_problem(&expected, &left_out).is_some());
        let stranger = ChequeTicks {
            present: vec![101, 102, 103],
            missing: vec![],
        };
        assert!(ticks_problem(&expected, &stranger).is_some());
        let twice = ChequeTicks {
            present: vec![101, 102],
            missing: vec![102],
        };
        assert!(ticks_problem(&expected, &twice).is_some());
        assert_eq!(ticks_problem(&[], &ChequeTicks::default()), None);
    }

    #[test]
    fn the_unticked_expected_cheques_are_missing() {
        let ticks = ChequeTicks::present_of(&[101, 102, 103], &[102, 999]);
        assert_eq!(ticks.present, vec![102]);
        assert_eq!(ticks.missing, vec![101, 103]);
    }
}
