//! The dispatches for the verticals (wl-inventory §6 `DispatchDirectory`):
//! what a vertical reads of a dispatch without depending on this crate. 07
//! Manufacturing guards `pick` and `dispatch` with the items on the lines
//! (`MFG.ACL.11`), follows a dispatch to its customer in a recall trace and
//! watches the export evidence of the dispatches that left (`MFG.RUL.08`).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::dubai_date;
use wl_kernel::inventory::{
    DISPATCH_SUBJECT_TYPE, DispatchDirectory, DispatchFacts, DispatchMaker, RedispatchLine,
    TraceFuture,
};
use wl_kernel::{PartyId, Subject};

use crate::dispatch::{
    Dispatch, DispatchStatus, NewDispatch, NewDispatchLine, create_draft, lines_of,
};
use crate::dispatch_events::event_lines;

/// The directory of wl-inventory's dispatches.
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryDispatches;

/// `dispatch` with its lines, as the kernel shows it.
async fn facts_of(db: &Db, dispatch: &Dispatch) -> Result<DispatchFacts> {
    let lines = lines_of(db, dispatch.id).await?;
    Ok(DispatchFacts {
        dispatch: dispatch.subject(),
        number: dispatch.number.clone().unwrap_or_default(),
        status: dispatch.status.as_str().to_owned(),
        source: dispatch.source(),
        recipient: dispatch.recipient_party_id.map(PartyId),
        dispatched_on: dispatch.dispatched_at.map(dubai_date),
        lines: event_lines(&lines),
    })
}

impl DispatchDirectory for InventoryDispatches {
    fn facts<'a>(
        &'a self,
        state: &'a AppState,
        dispatch: &'a Subject,
    ) -> TraceFuture<'a, Option<DispatchFacts>> {
        Box::pin(async move {
            if dispatch.subject_type != DISPATCH_SUBJECT_TYPE {
                return Ok(None);
            }
            match Dispatch::find(&state.db, dispatch.subject_id).await? {
                Some(found) => Ok(Some(facts_of(&state.db, &found).await?)),
                None => Ok(None),
            }
        })
    }

    fn of_source<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
    ) -> TraceFuture<'a, Vec<DispatchFacts>> {
        Box::pin(async move {
            let mut found = Vec::new();
            for dispatch in crate::dispatch::of_source(&state.db, source).await? {
                found.push(facts_of(&state.db, &dispatch).await?);
            }
            Ok(found)
        })
    }

    fn left_by<'a>(
        &'a self,
        state: &'a AppState,
        day: NaiveDate,
    ) -> TraceFuture<'a, Vec<DispatchFacts>> {
        Box::pin(async move {
            let left = Dispatch::query()
                .where_(Dispatch::STATUS.is_in([
                    DispatchStatus::Dispatched,
                    DispatchStatus::Delivered,
                    DispatchStatus::Lost,
                ]))
                .order_by(Dispatch::ID)
                .get(&state.db)
                .await?;
            let mut found = Vec::new();
            for dispatch in left {
                if dispatch
                    .dispatched_at
                    .map(dubai_date)
                    .is_some_and(|on| on <= day)
                {
                    found.push(facts_of(&state.db, &dispatch).await?);
                }
            }
            found.sort_by_key(|facts| (facts.dispatched_on, facts.dispatch.subject_id));
            Ok(found)
        })
    }
}

/// The kernel's [`DispatchMaker`]: the draft dispatch that delivers goods
/// again.
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryDispatchMaker;

impl DispatchMaker for InventoryDispatchMaker {
    fn redispatch<'a>(
        &'a self,
        state: &'a AppState,
        by: Option<i64>,
        original: &'a Subject,
        lines: &'a [RedispatchLine],
    ) -> TraceFuture<'a, Option<Subject>> {
        Box::pin(async move {
            if original.subject_type != DISPATCH_SUBJECT_TYPE {
                return Ok(None);
            }
            let Some(found) = Dispatch::find(&state.db, original.subject_id).await? else {
                return Ok(None);
            };
            let mut wanted = Vec::new();
            for line in lines_of(&state.db, found.id).await? {
                let Some(source_line) = line.source_line() else {
                    continue;
                };
                let Some(asked) = lines
                    .iter()
                    .find(|asked| asked.source_line == source_line && asked.quantity_milli > 0)
                else {
                    continue;
                };
                let mut redo =
                    NewDispatchLine::of(line.item_id, asked.quantity_milli).for_line(source_line);
                if let Some(lot) = line.lot_id {
                    redo = redo.lot(lot);
                }
                wanted.push(redo);
            }
            if wanted.is_empty() {
                return Ok(None);
            }
            let made = create_draft(
                &state.db,
                by,
                NewDispatch {
                    source: found.source(),
                    recipient: found.recipient_party_id.map(PartyId),
                    recipient_name: found.recipient_name.clone(),
                    address: found.address.clone(),
                    location_id: found.location_id,
                    lines: wanted,
                },
            )
            .await?;
            Ok(Some(made.subject()))
        })
    }
}
