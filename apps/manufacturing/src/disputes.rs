//! A quality dispute on an invoice becomes NCRs (07 Manufacturing §4.1 row
//! "An invoice is disputed", §4.5 step 5; BL-MFG-13, #394).
//!
//! wl-invoicing flags the invoice, links the ticket and pauses the reminders
//! (BL-INV-04, BL-INV-05) and announces [`InvoiceDisputed`] with the kind the
//! form asked, the reason and the disputed lines. For the kind *quality* this
//! vertical:
//!
//! - takes the disputed lines (all lines of the invoice when none was
//!   ticked), and for each the lots of the dispatch line it was drafted from
//!   (`MFG.RUL.04`), through the kernel's `DispatchLots`;
//! - raises one `open` NCR per lot with the buyer's text, or links the open
//!   NCR the lot has already (a lot on two disputed lines gets one); an item
//!   that is not lot-tracked gets one on the item and the dispatch line;
//! - links each NCR to the dispute and its ticket ([`NcrLink`]) and tells QC.
//!
//! The dispute is resolved in wl-invoicing; the NCR follows its own
//! lifecycle and is not closed by it. The buyer sees the ticket status only
//! ([`crate::portal`]), never the NCR. A kind other than quality, or a line
//! that bills no dispatch, creates nothing. The listener runs once per
//! dispute (a link row makes a repeat a no-op).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::{DISPATCH_LINE_SUBJECT_TYPE, LOT_SUBJECT_TYPE};
use wl_kernel::invoicing::{DISPUTE_SUBJECT_TYPE, InvoiceDisputed, invoicer};
use wl_kernel::lot_disposal::dispatch_lots;

use crate::ncr::{Ncr, NcrStatus, Origin};
use crate::ncrs::{self, NewNcr};
use crate::notices;
use crate::permissions;
use crate::texts::english;

/// The kind of dispute that opens NCRs.
pub const QUALITY: &str = "quality";

/// An NCR a dispute raised or found.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_ncr_links", index(ncr_id))]
pub struct NcrLink {
    /// The primary key.
    pub id: i64,
    /// The NCR.
    pub ncr_id: i64,
    /// The dispute, `invoicing.dispute:7`.
    pub dispute_id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// The billing dispute ticket the buyer follows.
    pub ticket_id: Option<i64>,
    /// The buyer's text.
    pub text: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The links of NCR `ncr_id`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn links_of(db: &Db, ncr_id: i64) -> Result<Vec<NcrLink>> {
    NcrLink::query()
        .where_(NcrLink::NCR_ID.eq(ncr_id))
        .order_by(NcrLink::ID)
        .get(db)
        .await
}

/// Turns the quality dispute of `event` into NCRs. Answers the NCRs raised or
/// linked.
///
/// # Errors
///
/// The database's, the kernel's and the notifier's errors.
pub async fn on_disputed(state: &AppState, event: &InvoiceDisputed) -> Result<Vec<Ncr>> {
    if event.kind.as_deref() != Some(QUALITY) {
        return Ok(Vec::new());
    }
    if NcrLink::query()
        .where_(NcrLink::DISPUTE_ID.eq(event.dispute))
        .count(&state.db)
        .await?
        > 0
    {
        return Ok(Vec::new());
    }
    let origins: Vec<Subject> = if event.lines.is_empty() {
        invoicer(state)
            .lines_of(&state.db, event.invoice)
            .await?
            .into_iter()
            .filter_map(|line| line.origin)
            .collect()
    } else {
        event
            .lines
            .iter()
            .filter_map(|line| line.origin.clone())
            .collect()
    };
    let dispute = Subject::new(DISPUTE_SUBJECT_TYPE, event.dispute);
    let mut found: Vec<Ncr> = Vec::new();
    for origin in origins {
        for sent in dispatch_lots(state).lots_of(state, &origin).await? {
            let existing = match &sent.lot {
                Some(lot) => {
                    Ncr::query()
                        .where_(Ncr::LOT_ID.eq(Some(lot.subject_id)))
                        .where_(Ncr::STATUS.eq(NcrStatus::Open))
                        .order_by(Ncr::ID)
                        .first(&state.db)
                        .await?
                }
                None => {
                    Ncr::query()
                        .where_(Ncr::LOT_ID.is_null())
                        .where_(Ncr::ITEM_ID.eq(sent.item.subject_id))
                        .where_(Ncr::SOURCE_TYPE.eq(Some(DISPATCH_LINE_SUBJECT_TYPE.to_owned())))
                        .where_(Ncr::SOURCE_ID.eq(Some(sent.line.subject_id)))
                        .where_(Ncr::STATUS.eq(NcrStatus::Open))
                        .first(&state.db)
                        .await?
                }
            };
            let ncr = match existing {
                Some(ncr) => ncr,
                None => {
                    ncrs::raise_by_system(
                        state,
                        NewNcr {
                            plant_id: 0,
                            lot: sent
                                .lot
                                .clone()
                                .filter(|lot| lot.subject_type == LOT_SUBJECT_TYPE),
                            item: Some(sent.item.clone()),
                            quantity_milli: sent.quantity_milli,
                            defect: event.reason.clone(),
                            cause: None,
                            origin: Origin::Dispute,
                            source: Some(if sent.lot.is_some() {
                                dispute.clone()
                            } else {
                                sent.line.clone()
                            }),
                        },
                    )
                    .await?
                }
            };
            if found.iter().any(|seen| seen.id == ncr.id) {
                continue;
            }
            NcrLink::create(
                &state.db,
                NcrLink {
                    ncr_id: ncr.id,
                    dispute_id: event.dispute,
                    invoice_id: event.invoice.0,
                    ticket_id: event.ticket.map(|ticket| ticket.0),
                    text: Some(event.reason.clone()),
                    ..NcrLink::default()
                },
            )
            .await?;
            found.push(ncr);
        }
    }
    if let Some(first) = found.first() {
        let numbers = found
            .iter()
            .map(|ncr| ncr.number.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let users = notices::holders(state, permissions::QC_ROLE, first.plant_id).await?;
        notices::tell(
            state,
            first.subject(),
            &first.number,
            &users,
            &english("manufacturing.dispute.title", &[("ncrs", &numbers)]),
            &english(
                "manufacturing.dispute.body",
                &[("ncrs", &numbers), ("text", &event.reason)],
            ),
        )
        .await?;
    }
    Ok(found)
}

/// The listener.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: InvoiceDisputed, state: AppState| async move {
        on_disputed(&state, &event).await.map(|_| ())
    });
}
