//! A till's cash handover from a closed till session (wl-payments §4 *Cash
//! handover*, mode *Till session*, §5.5 step 1, §6; BL-PAY-19, issue #112).
//!
//! The cash of a till is counted in the wl-sales till session: its `close`
//! records the expected and the counted cash and approves the variance there.
//! [`create_from_session`] runs on `TillSessionClosed` and
//!
//! - creates the till's handover with the session's expected cash, the till
//!   as the holder ([`CashHandover::holder_till`]) and the session's cashier
//!   as the responsible person (recoveries are taken from the cashier, the
//!   holder stays the till). The period is the session (`sales.till_session`);
//!   `holder_user_id` is the cashier too, the person the platform can notify;
//! - runs the system transition `count_till` (or `count_till_short`) with the
//!   counted cash: no user counts the till again (the guard "counter is not
//!   the holder" is met by the session's close and review in wl-sales). A
//!   shortage then follows the path of BL-PAY-17: `CashShortage` (with the
//!   till and the session), the notices, `recover` and `write_off`.
//!
//! One handover is made per till session: the same event again creates and
//! counts nothing. The handover is never escalated by `FIN.RUL.22` (it is
//! counted when it is made).

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::custody::{HandoverKind, HandoverStatus, PeriodKind};
use wl_kernel::sales::TillSessionClosed;

use crate::cash_count::{COUNT_TILL, COUNT_TILL_SHORT, CountInput};
use crate::cash_handover::{CashEntry, CashHandover, CashHolder};

/// The handover of the till session `session`, if one was made.
///
/// # Errors
///
/// Database errors.
pub async fn of_session(db: &Db, session: i64) -> Result<Option<CashHandover>> {
    CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::Period))
        .where_(
            CashHandover::PERIOD_TYPE.eq(wl_kernel::sales::TILL_SESSION_SUBJECT_TYPE.to_owned()),
        )
        .where_(CashHandover::PERIOD_ID.eq(session))
        .order_by(CashHandover::ID)
        .first(db)
        .await
}

/// Creates the till's handover of a closed session and counts it. Returns
/// the handover as it is now (`counted`, or `short` with the difference);
/// the one already made when the event comes twice.
///
/// # Errors
///
/// Database errors; a negative amount in the event (422).
pub async fn create_from_session(
    state: &AppState,
    event: &TillSessionClosed,
) -> Result<CashHandover> {
    if event.expected < 0 || event.counted < 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "A till session's cash cannot be negative.".to_owned(),
        ));
    }
    let db = &state.db;
    let subject = event.session_subject();
    let handover = match of_session(db, event.session).await? {
        Some(existing) => existing,
        None => {
            let holder = CashHolder::ensure(
                db,
                event.cashier_user,
                event.location.as_deref(),
                event.legal_entity_id,
            )
            .await?;
            let mut tx = db.begin().await?;
            let mut created = CashHandover {
                kind: HandoverKind::Period,
                holder_id: holder.id,
                holder_user_id: event.cashier_user,
                responsible_user_id: event.cashier_user,
                holder_till: Some(event.till.clone()),
                period_kind: PeriodKind::TillSession,
                period_type: Some(subject.subject_type.clone()),
                period_id: Some(subject.subject_id),
                period_date: event.date,
                escalate: false,
                expected: event.expected,
                currency: "AED".to_owned(),
                location: event.location.clone().or(holder.location.clone()),
                legal_entity_id: event.legal_entity_id.or(holder.legal_entity_id),
                status: HandoverStatus::Open,
                ..Default::default()
            };
            created
                .insert_in_state(&mut tx, HandoverStatus::Open, "Till session closed")
                .await?;
            let mut entry = CashEntry {
                handover_id: created.id,
                amount: event.expected,
                source_type: subject.subject_type.clone(),
                source_id: subject.subject_id,
                note: Some(event.till.clone()),
                ..Default::default()
            };
            entry.insert(&mut tx).await?;
            tx.commit().await?;
            created
        }
    };
    if handover.status != HandoverStatus::Open {
        return Ok(handover);
    }
    let name = if event.counted < handover.expected {
        COUNT_TILL_SHORT
    } else {
        COUNT_TILL
    };
    let input = CountInput {
        counted: event.counted,
        holder_present: None,
        holder_absent: false,
        note: Some(format!("Till session {} of {}", event.session, event.till)),
    };
    let _outcome = handover
        .transition(name)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    CashHandover::find_or_404(db, handover.id).await
}

/// Demo rows (DEMO-DATA.md §06): two closed till sessions of till `T1` for
/// the cashier `cashier_user`: session 9001 AED 50.00 short (a `short`
/// handover to recover or write off) and session 9002 balanced (`counted`).
/// Idempotent: a session that has its handover is skipped. Returns how many
/// handovers were made.
///
/// # Errors
///
/// Database errors.
pub async fn demo(
    state: &AppState,
    cashier_user: i64,
    date: renox::chrono::NaiveDate,
) -> Result<usize> {
    let mut made = 0;
    for (session, expected, counted) in [(9_001, 400_000, 395_000), (9_002, 250_000, 250_000)] {
        if of_session(&state.db, session).await?.is_some() {
            continue;
        }
        create_from_session(
            state,
            &TillSessionClosed {
                session,
                till: "T1".to_owned(),
                cashier_user,
                date,
                expected,
                counted,
                location: None,
                legal_entity_id: None,
            },
        )
        .await?;
        made += 1;
    }
    Ok(made)
}

/// Registers the listener of `TillSessionClosed`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: TillSessionClosed, state: AppState| async move {
        create_from_session(&state, &event).await.map(|_| ())
    });
}
