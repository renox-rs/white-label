//! Raising a later cash shortage against a holder (wl-payments §4 *Cash
//! handover* `raise_shortage`, §5.5 step 5, §6 `CashShortage`, `FIN.ACL.05`,
//! issue #116).
//!
//! Some shortages show up after the holder's handover was counted: in 03 a
//! delivery is reversed days later with liability *driver*, so the COD the
//! driver handed over was never collected. The operations manager (`cash.count`)
//! raises the amount: a new handover of kind *later shortage*, `short` from the
//! start with expected = the amount and counted = 0, linked to its source
//! (a [`Subject`]), notifying the holder and the holder's manager with
//! `CashShortage` carrying the source. It is then recovered or written off
//! like any other shortage ([`crate::cash_settle`]).
//!
//! - The raiser is never the holder (`FIN.ACL.05`).
//! - While the holder's handover that took the cash of the source is still
//!   `open`, nothing new is made: the amount is added to its expected total
//!   and shows at its count.
//! - A second later shortage for the same source is refused while the first is
//!   still `short`.
//! - The handover the cash first came in on is never changed.
//! - A client's COD remittance is never reduced by the shortage (03 §9): that
//!   is apps/logistics, which does not read this module.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::custody::{HandoverKind, HandoverStatus, PeriodKind};
use wl_kernel::documents::today;

use crate::cash_count::shortage_event;
use crate::cash_handover::{CashEntry, CashHandover, CashHolder};
use crate::cash_notices;
use crate::permissions;

/// What the manager gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaiseShortage {
    /// The holder the shortage is raised against, a staff user.
    pub holder_user: i64,
    /// The amount, in fils.
    pub amount: i64,
    /// Why.
    pub reason: String,
    /// The record that caused it (the reversed delivery).
    pub source: Subject,
}

fn guard(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// Why `request` cannot be raised by `user_id`, if it cannot: no reason, an
/// amount not above 0, or the raiser is the holder (`FIN.ACL.05`).
pub(crate) fn raise_problem(request: &RaiseShortage, user_id: i64) -> Option<String> {
    if request.reason.trim().is_empty() {
        return Some("A reason is required.".to_owned());
    }
    if request.amount <= 0 {
        return Some("The shortage is above 0.".to_owned());
    }
    if user_id == request.holder_user {
        return Some("A shortage is raised by someone other than its holder (FIN.ACL.05).".to_owned());
    }
    None
}

/// `user` (`cash.count`) raises a later shortage against the holder.
/// Returns the new handover, `short`; or, while the holder's handover for that
/// cash is still `open`, that handover with the amount added to its expected
/// total.
///
/// # Errors
///
/// `Forbidden` without `cash.count`; `Guard` (422) without a reason, with an
/// amount that is not above 0, for the holder themselves, or while a later
/// shortage for the same source is still `short`.
pub async fn raise_shortage(
    state: &AppState,
    user: &AuthUser,
    request: &RaiseShortage,
) -> std::result::Result<CashHandover, TransitionError> {
    if !user.has_permission(permissions::CASH_COUNT) {
        return Err(TransitionError::Forbidden);
    }
    if let Some(problem) = raise_problem(request, user.id) {
        return Err(guard(problem));
    }
    let db = &state.db;
    let open = CashHandover::query()
        .where_(CashHandover::KIND.eq(HandoverKind::LaterShortage))
        .where_(CashHandover::SOURCE_TYPE.eq(request.source.subject_type.clone()))
        .where_(CashHandover::SOURCE_ID.eq(request.source.subject_id))
        .where_(CashHandover::STATUS.eq(HandoverStatus::Short))
        .count(db)
        .await?;
    if open > 0 {
        return Err(guard(
            "A later shortage for this source is already open: recover or write it off first.",
        ));
    }
    // The handover that took the cash of the source, if there is one.
    let entries = CashEntry::of_source(db, &request.source).await?;
    let mut first: Option<CashHandover> = None;
    for entry in entries.iter().filter(|e| e.amount > 0) {
        let found = CashHandover::find_or_404(db, entry.handover_id).await?;
        if found.holder_user_id == request.holder_user {
            first = Some(found);
            break;
        }
    }
    if let Some(open_one) = first.as_ref().filter(|h| h.status == HandoverStatus::Open) {
        // Still with the holder: it shows at the count.
        let mut tx = db.begin().await?;
        let mut entry = CashEntry {
            handover_id: open_one.id,
            amount: request.amount,
            source_type: request.source.subject_type.clone(),
            source_id: request.source.subject_id,
            note: Some(request.reason.trim().to_owned()),
            ..Default::default()
        };
        entry.insert(&mut tx).await?;
        let rows = CashHandover::query()
            .where_(CashHandover::ID.eq(open_one.id))
            .where_(CashHandover::STATUS.eq(HandoverStatus::Open))
            .increment(&mut tx, "expected", request.amount)
            .await?;
        if rows == 0 {
            return Err(guard("The handover was counted meanwhile: raise it again."));
        }
        tx.commit().await?;
        return Ok(CashHandover::find_or_404(db, open_one.id).await?);
    }
    let holder = CashHolder::ensure(
        db,
        request.holder_user,
        first.as_ref().and_then(|h| h.location.as_deref()),
        first.as_ref().and_then(|h| h.legal_entity_id),
    )
    .await?;
    let mut created = CashHandover {
        kind: HandoverKind::LaterShortage,
        holder_id: holder.id,
        holder_user_id: request.holder_user,
        responsible_user_id: first
            .as_ref()
            .map_or(request.holder_user, |h| h.responsible_user_id),
        period_kind: first.as_ref().map_or(PeriodKind::Day, |h| h.period_kind),
        period_type: first.as_ref().and_then(|h| h.period_type.clone()),
        period_id: first.as_ref().and_then(|h| h.period_id),
        period_date: first.as_ref().map_or_else(today, |h| h.period_date),
        escalate: false,
        expected: request.amount,
        counted: 0,
        difference: request.amount,
        currency: "AED".to_owned(),
        location: holder.location.clone(),
        legal_entity_id: holder.legal_entity_id,
        source_type: Some(request.source.subject_type.clone()),
        source_id: Some(request.source.subject_id),
        reason: Some(request.reason.trim().to_owned()),
        created_by: Some(user.id),
        status: HandoverStatus::Short,
        ..Default::default()
    };
    let mut tx = db.begin().await?;
    created
        .insert_in_state(&mut tx, HandoverStatus::Short, request.reason.trim())
        .await?;
    tx.commit().await?;
    state.emit(shortage_event(&created)).await?;
    cash_notices::shortage(state, &created).await?;
    Ok(CashHandover::find_or_404(db, created.id).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> RaiseShortage {
        RaiseShortage {
            holder_user: 5,
            amount: 30_000,
            reason: "Shipment reversed".to_owned(),
            source: Subject::new("logistics.shipment", 9),
        }
    }

    #[test]
    fn a_later_shortage_needs_a_reason_an_amount_and_another_raiser() {
        assert_eq!(raise_problem(&request(), 6), None);
        assert!(raise_problem(&request(), 5).is_some());
        let mut blank = request();
        blank.reason = "  ".to_owned();
        assert!(raise_problem(&blank, 6).is_some());
        let mut nothing = request();
        nothing.amount = 0;
        assert!(raise_problem(&nothing, 6).is_some());
    }
}
