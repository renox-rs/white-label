//! Moving cash between drivers on a run hand-over (wl-payments §4 *Cash
//! handover* `hand_over`, §6 *Events consumed* `RunHandedOver`, `FIN.CSH.01`;
//! issue #113).
//!
//! When `wl-delivery` hands open stops over to another driver's run, the
//! receiving driver confirms in the driver app the cash counted (and the
//! cheques received). [`hand_over`] runs on `RunHandedOver`:
//!
//! - the counted cash **leaves the giver's expected amount** (a negative
//!   [`CashEntry`] of the hand-over) and **joins the receiver's open
//!   handover** of the receiving run (opened for the receiver's period when
//!   none is open), each payment keeping its collector: the giver's entries
//!   stay as they are, the move is one more entry on each side;
//! - what the giver should hand over and the receiver did not count **stays
//!   on the giver's handover**, which shows it as a shortage at its `count`;
//! - when the receiver counted **more** than the giver's expected amount, the
//!   giver's whole expected amount moves and the excess is an *overage line
//!   from the hand-over* on the receiver's handover: added to its expected
//!   cash and held as a liability, like a counting overage (§12 question 4),
//!   so the giver's handover keeps nothing for it;
//! - the giver, the receiver and the dispatcher are told
//!   ([`crate::cash_notices::handed_over`]).
//!
//! A hand-over to oneself is refused, and the event twice moves nothing the
//! second time: the entries of the hand-over are its key
//! (`delivery.run_hand_over:{id}`, and `delivery.run_hand_over_overage:{id}`
//! for the overage line), so a call that stopped half-way is finished by the
//! next one with the amounts it had worked out. The cheques the receiver
//! confirmed travel in the event, but cheques are not in the handovers yet
//! (`CashShortage::missing_cheques` is empty until they are): they stay
//! expected, as the issue says of cheques the receiver did not confirm.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::custody::{CashCollection, HandoverId, HandoverStatus};
use wl_kernel::delivery::RunHandedOver;

use crate::cash_custody::collect;
use crate::cash_handover::{CashEntry, CashHandover, CashPosting, PostingKind};
use crate::cash_notices;

/// The subject type of the entries a hand-over leaves on the two handovers.
pub const SOURCE_TYPE: &str = "delivery.run_hand_over";

/// The subject type of the overage line a hand-over leaves on the receiver's
/// handover.
pub const OVERAGE_SOURCE_TYPE: &str = "delivery.run_hand_over_overage";

fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// What a hand-over moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// What left the giver's expected amount and joined the receiver's, in
    /// fils.
    pub moved: i64,
    /// What the receiver counted above the giver's expected amount, in
    /// fils: the overage line of the receiver's handover.
    pub overage: i64,
    /// The receiver's handover, when any cash joined it.
    pub receiver: Option<HandoverId>,
    /// The giver's handover, when it holds the run's cash.
    pub giver: Option<HandoverId>,
    /// Whether an earlier call had done it all already.
    pub already: bool,
}

/// The entries of the hand-over `id`.
fn source(id: i64) -> Subject {
    Subject::new(SOURCE_TYPE, id)
}

/// The overage line of the hand-over `id`.
fn overage_source(id: i64) -> Subject {
    Subject::new(OVERAGE_SOURCE_TYPE, id)
}

/// Moves the cash `event` carries from the giver's handover of the run to the
/// receiver's.
///
/// # Errors
///
/// 422 for a hand-over to the same driver, a negative amount, or a giver's
/// handover that was counted already; database errors.
pub async fn hand_over(state: &AppState, event: &RunHandedOver) -> Result<Moved> {
    if event.to_driver == event.from_driver {
        return Err(refused("A driver cannot hand cash over to themselves."));
    }
    if event.cash_counted < 0 {
        return Err(refused("The cash counted cannot be negative."));
    }
    let db = &state.db;
    let key = source(event.hand_over);
    let overage_key = overage_source(event.hand_over);
    let entries = CashEntry::of_source(db, &key).await?;
    let overage_entries = CashEntry::of_source(db, &overage_key).await?;
    let giver = CashHandover::of_period_record(db, event.from_driver, &event.run.subject()).await?;
    let giver_id = giver.as_ref().map(CashHandover::handover_id);
    // Done already: the giver gave up its part, or only an overage line was
    // ever needed and it is there.
    if entries.iter().any(|entry| entry.amount < 0)
        || (entries.is_empty() && !overage_entries.is_empty())
    {
        let receiver = match entries.first() {
            Some(entry) => Some(HandoverId(entry.handover_id)),
            None => overage_entries
                .first()
                .map(|entry| HandoverId(entry.handover_id)),
        };
        return Ok(Moved {
            moved: entries.iter().map(|entry| entry.amount.abs()).max().unwrap_or(0),
            overage: overage_entries.iter().map(|entry| entry.amount).sum(),
            receiver,
            giver: giver_id,
            already: true,
        });
    }
    if let Some(giver) = &giver
        && giver.status != HandoverStatus::Open
        && entries.is_empty()
        && event.cash_counted > 0
    {
        return Err(refused(
            "The giving driver's cash of this run was already counted.",
        ));
    }
    // What the giver can give: the amount a first call worked out, else what
    // the giver's handover expects now.
    let moved = match entries.iter().find(|entry| entry.amount > 0) {
        Some(entry) => entry.amount,
        None => event
            .cash_counted
            .min(giver.as_ref().map_or(0, |giver| giver.expected).max(0)),
    };
    let overage = event.cash_counted - moved;
    let mut receiver = None;
    if moved > 0 {
        receiver = Some(
            collect(
                state,
                CashCollection::new(
                    event.to_driver,
                    event.to_period.clone(),
                    moved,
                    key.clone(),
                ),
            )
            .await?,
        );
    }
    if overage > 0 {
        let new_line = overage_entries.is_empty();
        let id = collect(
            state,
            CashCollection::new(
                event.to_driver,
                event.to_period.clone(),
                overage,
                overage_key.clone(),
            ),
        )
        .await?;
        receiver = Some(id);
        if new_line {
            // Held as a liability, not as income, until the accountant
            // decides (§12 question 4), as a counting overage is.
            let handover = CashHandover::find_or_404(db, id.0).await?;
            let mut posting = CashPosting {
                handover_id: handover.id,
                kind: PostingKind::OverageLiability,
                amount: overage,
                location: handover.location.clone(),
                legal_entity_id: handover.legal_entity_id,
                ..Default::default()
            };
            posting.insert(db).await?;
        }
    }
    if moved > 0
        && let Some(giver) = &giver
    {
        let mut tx = db.begin().await?;
        let mut taken = CashEntry {
            handover_id: giver.id,
            amount: -moved,
            source_type: key.subject_type.clone(),
            source_id: key.subject_id,
            note: Some("handed over to another driver".to_owned()),
            ..Default::default()
        };
        taken.insert(&mut tx).await?;
        // The status guard keeps the cash out of a handover counted meanwhile.
        let rows = CashHandover::query()
            .where_(CashHandover::ID.eq(giver.id))
            .where_(CashHandover::STATUS.eq(HandoverStatus::Open))
            .increment(&mut tx, "expected", -moved)
            .await?;
        if rows == 0 {
            return Err(refused(
                "The giving driver's cash of this run was already counted.",
            ));
        }
        tx.commit().await?;
    }
    cash_notices::handed_over(state, event, moved, overage, giver_id).await?;
    Ok(Moved {
        moved,
        overage,
        receiver,
        giver: giver_id,
        already: false,
    })
}

/// Registers the listener of `RunHandedOver`.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: RunHandedOver, state: AppState| async move {
        hand_over(&state, &event).await.map(|_| ())
    });
}
