//! Cash custody (wl-payments §3 *Cash holder*, §5.5 steps 1-2, `FIN.CSH.01`;
//! base for #110, #113, #114, #116): how other modules put cash into a
//! holder's handover through the kernel's
//! [`CashCustody`](wl_kernel::custody::CashCustody).
//!
//! - [`collect`] adds the cash a holder collected for a source (a delivery)
//!   to the holder's handover of the period, opening it (`open`) with the
//!   first cash. One source adds its cash once; a handover that is no longer
//!   `open` takes no more cash (it was counted).
//! - [`release`] takes the cash collected for a source out again while the
//!   handover is `open` (a delivery reversed before the count). After the
//!   count a later shortage is raised instead ([`crate::cash_later`]).
//! - [`end_period`] sets when a period ends (a run's return or abort, or its
//!   planned end, whichever is earlier), which starts `FIN.RUL.22`
//!   ([`crate::cash_escalation`]).
//!
//! The holder is a staff user, per person and per day unless the period says
//! otherwise; the row is made with the first collection.

use renox::state_machine::StateMachine;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::custody::{
    CashCollection, CashCustody, CustodyFuture, HandoverId, HandoverKind, HandoverStatus,
    HandoverSummary,
};

use crate::cash_handover::{CashEntry, CashHandover, CashHolder};

fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The kernel contract's implementation, provided by the module.
#[derive(Debug, Clone, Copy, Default)]
pub struct Custody;

impl CashCustody for Custody {
    fn collect<'a>(
        &'a self,
        state: &'a AppState,
        collection: CashCollection,
    ) -> CustodyFuture<'a, Option<HandoverId>> {
        Box::pin(async move { collect(state, collection).await.map(Some) })
    }

    fn release<'a>(&'a self, state: &'a AppState, source: Subject) -> CustodyFuture<'a, bool> {
        Box::pin(async move { release(state, &source).await })
    }

    fn end_period<'a>(
        &'a self,
        state: &'a AppState,
        holder_user: i64,
        period: Subject,
        ends_at: DateTime,
    ) -> CustodyFuture<'a, ()> {
        Box::pin(async move { end_period(state, holder_user, &period, ends_at).await })
    }

    fn handover_of<'a>(
        &'a self,
        db: &'a Db,
        holder_user: i64,
        period: &'a Subject,
    ) -> CustodyFuture<'a, Option<HandoverSummary>> {
        Box::pin(async move {
            Ok(CashHandover::of_period_record(db, holder_user, period)
                .await?
                .map(|handover| handover.summary()))
        })
    }

    fn find<'a>(
        &'a self,
        db: &'a Db,
        id: HandoverId,
    ) -> CustodyFuture<'a, Option<HandoverSummary>> {
        Box::pin(async move { Ok(CashHandover::find(db, id.0).await?.map(|h| h.summary())) })
    }
}

/// Adds `collection` to the holder's handover of its period.
///
/// # Errors
///
/// 422 for an amount of 0 or less, or when the holder's handover of that
/// period was already counted; database errors.
pub async fn collect(state: &AppState, collection: CashCollection) -> Result<HandoverId> {
    if collection.amount <= 0 {
        return Err(refused("The cash collected is above 0."));
    }
    let db = &state.db;
    // One source adds its cash once.
    let earlier = CashEntry::of_source(db, &collection.source).await?;
    if let Some(entry) = earlier.iter().find(|entry| entry.amount > 0) {
        return Ok(HandoverId(entry.handover_id));
    }
    let holder = CashHolder::ensure(
        db,
        collection.holder_user,
        collection.location.as_deref(),
        collection.legal_entity_id,
    )
    .await?;
    let existing = CashHandover::of_period(db, collection.holder_user, &collection.period).await?;
    let mut tx = db.begin().await?;
    let handover = match existing {
        Some(existing) if existing.status != HandoverStatus::Open => {
            return Err(refused(
                "The holder's cash of this period was already counted: raise a later shortage.",
            ));
        }
        Some(existing) => existing,
        None => {
            let mut created = CashHandover {
                kind: HandoverKind::Period,
                holder_id: holder.id,
                holder_user_id: collection.holder_user,
                responsible_user_id: collection.holder_user,
                period_kind: collection.period.kind,
                period_type: collection
                    .period
                    .subject
                    .as_ref()
                    .map(|s| s.subject_type.clone()),
                period_id: collection.period.subject.as_ref().map(|s| s.subject_id),
                period_date: collection.period.date,
                period_end: collection.period.ends_at,
                escalate: collection.period.escalate,
                currency: "AED".to_owned(),
                location: holder.location.clone(),
                legal_entity_id: holder.legal_entity_id,
                status: HandoverStatus::Open,
                ..Default::default()
            };
            created
                .insert_in_state(&mut tx, HandoverStatus::Open, "Cash collected")
                .await?;
            created
        }
    };
    let mut entry = CashEntry {
        handover_id: handover.id,
        amount: collection.amount,
        source_type: collection.source.subject_type.clone(),
        source_id: collection.source.subject_id,
        ..Default::default()
    };
    entry.insert(&mut tx).await?;
    // The status guard keeps the cash out of a handover counted meanwhile.
    let rows = CashHandover::query()
        .where_(CashHandover::ID.eq(handover.id))
        .where_(CashHandover::STATUS.eq(HandoverStatus::Open))
        .increment(&mut tx, "expected", collection.amount)
        .await?;
    if rows == 0 {
        return Err(refused(
            "The holder's cash of this period was already counted: raise a later shortage.",
        ));
    }
    tx.commit().await?;
    Ok(handover.handover_id())
}

/// Takes the cash collected for `source` out of its `open` handover again.
/// `false` when nothing was collected for it (or it was taken out already).
///
/// # Errors
///
/// 422 once the handover was counted: raise a later shortage instead.
pub async fn release(state: &AppState, source: &Subject) -> Result<bool> {
    let db = &state.db;
    let entries = CashEntry::of_source(db, source).await?;
    let Some(entry) = entries.iter().find(|entry| entry.amount > 0) else {
        return Ok(false);
    };
    if entries.iter().any(|e| e.amount < 0) {
        return Ok(false);
    }
    let handover = CashHandover::find_or_404(db, entry.handover_id).await?;
    if handover.status != HandoverStatus::Open {
        return Err(refused(
            "The holder's cash was already counted: raise a later shortage instead.",
        ));
    }
    let mut tx = db.begin().await?;
    let mut taken_out = CashEntry {
        handover_id: handover.id,
        amount: -entry.amount,
        source_type: source.subject_type.clone(),
        source_id: source.subject_id,
        note: Some("released".to_owned()),
        ..Default::default()
    };
    taken_out.insert(&mut tx).await?;
    let rows = CashHandover::query()
        .where_(CashHandover::ID.eq(handover.id))
        .where_(CashHandover::STATUS.eq(HandoverStatus::Open))
        .increment(&mut tx, "expected", -entry.amount)
        .await?;
    if rows == 0 {
        return Err(refused(
            "The holder's cash was already counted: raise a later shortage instead.",
        ));
    }
    tx.commit().await?;
    Ok(true)
}

/// Sets the end of the period `period` of `holder_user`: the earlier of the
/// end recorded and `ends_at`.
///
/// # Errors
///
/// Database errors.
pub async fn end_period(
    state: &AppState,
    holder_user: i64,
    period: &Subject,
    ends_at: DateTime,
) -> Result {
    let Some(handover) = CashHandover::of_period_record(&state.db, holder_user, period).await?
    else {
        return Ok(());
    };
    if handover.status != HandoverStatus::Open {
        return Ok(());
    }
    let end = match handover.period_end {
        Some(recorded) if recorded <= ends_at => recorded,
        _ => ends_at,
    };
    CashHandover::query()
        .where_(CashHandover::ID.eq(handover.id))
        .update(&state.db, &[("period_end", &Some(end))])
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_custody_is_a_cash_custody() {
        fn takes(_: impl CashCustody) {}
        takes(Custody);
    }
}
