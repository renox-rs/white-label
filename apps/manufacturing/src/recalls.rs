//! A recalled lot traced to the customers of 07 Manufacturing (§4.4 step 8,
//! §9 *Traceability and recall*; BL-MFG-10, #391).
//!
//! The recall is wl-inventory's: its owner opens it, every lot in scope is
//! held, and its trace follows the movements of the lots through the work
//! orders that used them to the lots they made and through the dispatches to
//! the customers. Three things are this vertical's:
//!
//! - [`ProductionParties`], the app's `TracePartyResolver`, which says whose
//!   a subject is, always through the dispatch or the work order (never from
//!   the item or the lot alone):
//!   - a **dispatch** is its customer, the recipient the dispatch was made
//!     for (the customer of its sales order); a dispatch that has not left
//!     (`draft`, `picked`) is *not shipped*, one that left is *shipped*; a
//!     `returned`, `discarded` or `cancelled` one is not listed (the goods
//!     are back on site, and held);
//!   - a **work order** for an order not yet delivered is that order's
//!     customer, *not shipped*; a work order for stock has no party (its
//!     output lots are in the scope all the same). The customer comes from
//!     the kernel's `OrderLines`; an order whose line is cancelled or has
//!     nothing open is delivered or gone, and gives no party;
//!   - the notice goes to every active portal buyer of the customer
//!     ([`ProductionParties::addressees`]), else to the customer itself,
//!     which reaches its main contact email. The resolver sends nothing.
//! - [`on_traced`]: each dispatch of the trace that has not left is
//!   **flagged for re-pick** once per recall ([`RepickFlag`]) and the
//!   storekeeper gets a task to pick it again from lots that are not held;
//!   a dispatch that already left (`dispatched`) is listed for the recall
//!   owner to bring back ([`TraceRow::bring_back`]), by hand: the return is
//!   wl-delivery's.
//! - [`trace`], the list the recall owner reads at `/manufacturing/recalls/{id}`
//!   ([`crate::screens::recall_page`]): per customer, the work orders and
//!   dispatches with *shipped* or *not shipped*, the lot, the quantity and
//!   whether the notice went out.
//!
//! Decisions: the dispatch's recipient is taken as the customer (the
//! dispatch is made for the order's customer and the order has no other
//! party on it); the storekeeper is found like the supervisor of a hold,
//! the holders of the role, else whom the kernel picks; a re-run of the trace
//! flags a dispatch once (a completed task is not reopened); authority
//! notification is not built (AU-20).

use renox::auth::permissions::users_with_role_in;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::inventory::{
    DELIVERED, DISPATCH_SUBJECT_TYPE, NOT_SHIPPED, RECALL_SUBJECT_TYPE, RecallTraced, SHIPPED,
    TraceFuture, TraceParty, TracePartyResolver, dispatch_directory, lot_trace,
};
use wl_kernel::lot_disposal::lot_disposal;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::portal::portal_users;
use wl_kernel::production::order_lines;
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};
use wl_kernel::{PartyId, Subject};

use crate::permissions;
use crate::texts::english;
use crate::work_order::{SUBJECT_TYPE as WORK_ORDER, WorkOrder, WorkOrderSource};

/// The dispatch states from which the goods are back on site.
const BACK: [&str; 3] = ["returned", "discarded", "cancelled"];

/// A dispatch a recall stopped, flagged for a re-pick.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_repick_flags",
    unique(recall_id, dispatch_id),
    index(dispatch_id)
)]
pub struct RepickFlag {
    /// The primary key.
    pub id: i64,
    /// The recall, `inventory.recall:2`.
    pub recall_id: i64,
    /// The dispatch, `inventory.dispatch:3`.
    pub dispatch_id: i64,
    /// The number of the dispatch, `DSP-2026-0001`.
    pub number: String,
    /// The storekeeper's task.
    pub ticket_id: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RepickFlag {
    /// The flag of `dispatch_id` in recall `recall_id`.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of(db: &Db, recall_id: i64, dispatch_id: i64) -> Result<Option<RepickFlag>> {
        RepickFlag::query()
            .where_(RepickFlag::RECALL_ID.eq(recall_id))
            .where_(RepickFlag::DISPATCH_ID.eq(dispatch_id))
            .first(db)
            .await
    }
}

/// The source key of the storekeeper's task to re-pick `dispatch_id` for
/// recall `recall_id`.
pub fn repick_key(recall_id: i64, dispatch_id: i64) -> String {
    format!("manufacturing.repick:{recall_id}:{dispatch_id}")
}

/// The app's `TracePartyResolver`: a dispatch or a work order is its
/// customer.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProductionParties;

/// The customer of the sales order a work order serves, while its goods are
/// still to ship.
async fn order_customer(state: &AppState, order: &WorkOrder) -> Result<Option<PartyId>> {
    let (Some(sales), Some(line)) = (order.order(), order.sales_order_line_id) else {
        return Ok(None);
    };
    let Some(info) = order_lines(state).line(state, sales, line).await? else {
        return Ok(None);
    };
    if info.cancelled || info.open_milli <= 0 {
        return Ok(None);
    }
    Ok(info.customer)
}

impl TracePartyResolver for ProductionParties {
    fn party<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> TraceFuture<'a, TraceParty> {
        Box::pin(async move {
            if subject.subject_type == DISPATCH_SUBJECT_TYPE {
                let Some(facts) = dispatch_directory(state).facts(state, subject).await? else {
                    return Ok(TraceParty::Unknown);
                };
                if BACK.contains(&facts.status.as_str()) {
                    return Ok(TraceParty::NotAffected);
                }
                return Ok(facts
                    .recipient
                    .map_or(TraceParty::Unknown, TraceParty::Party));
            }
            if subject.subject_type == WORK_ORDER {
                let Some(order) = WorkOrder::find(&state.db, subject.subject_id).await? else {
                    return Ok(TraceParty::Unknown);
                };
                if order.source != WorkOrderSource::SalesOrder {
                    return Ok(TraceParty::Unknown);
                }
                return Ok(order_customer(state, &order)
                    .await?
                    .map_or(TraceParty::Unknown, TraceParty::Party));
            }
            Ok(TraceParty::Unknown)
        })
    }

    fn delivery_status<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> TraceFuture<'a, Option<String>> {
        Box::pin(async move {
            if subject.subject_type == WORK_ORDER {
                return Ok(Some(NOT_SHIPPED.to_owned()));
            }
            if subject.subject_type != DISPATCH_SUBJECT_TYPE {
                return Ok(None);
            }
            let Some(facts) = dispatch_directory(state).facts(state, subject).await? else {
                return Ok(None);
            };
            Ok(match facts.status.as_str() {
                "draft" | "picked" => Some(NOT_SHIPPED.to_owned()),
                "delivered" => Some(DELIVERED.to_owned()),
                "dispatched" | "lost" => Some(SHIPPED.to_owned()),
                _ => None,
            })
        })
    }

    fn addressees<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> TraceFuture<'a, Vec<NotifyRecipient>> {
        Box::pin(async move {
            let buyers = portal_users(state)
                .users_for(&state.db, party, wl_kernel::documents::today())
                .await?;
            if buyers.is_empty() {
                return Ok(vec![NotifyRecipient::Party { id: party }]);
            }
            Ok(buyers
                .into_iter()
                .map(|buyer| NotifyRecipient::User { id: buyer.user_id })
                .collect())
        })
    }
}

/// The user a re-pick task goes to: a storekeeper of a plant (the dispatch
/// names no plant, so the first plant that has one), else whom the kernel
/// picks for the role.
async fn storekeeper(state: &AppState) -> Result<Option<i64>> {
    for plant in crate::plant::Plant::query()
        .order_by(crate::plant::Plant::ID)
        .get(&state.db)
        .await?
    {
        if let Some(user) = users_with_role_in(
            &state.db,
            permissions::STOREKEEPER_ROLE,
            &permissions::plant_scope(plant.id),
        )
        .await?
        .first()
        {
            return Ok(Some(user.id));
        }
    }
    role_tasks::assignee(&state.db, permissions::STOREKEEPER_ROLE).await
}

/// A recall was traced: every dispatch of its lines that has not left is
/// flagged for a re-pick, once per recall, with a task for the storekeeper.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_traced(state: &AppState, event: &RecallTraced) -> Result {
    let recall_id = event.recall.subject_id;
    let mut seen = Vec::new();
    for line in &event.lines {
        let Some(subject) = line
            .subject
            .as_ref()
            .filter(|subject| subject.subject_type == DISPATCH_SUBJECT_TYPE)
        else {
            continue;
        };
        if line.delivery_status.as_deref() != Some(NOT_SHIPPED) || seen.contains(subject) {
            continue;
        }
        seen.push(subject.clone());
        if RepickFlag::of(&state.db, recall_id, subject.subject_id)
            .await?
            .is_some()
        {
            continue;
        }
        let number = dispatch_directory(state)
            .facts(state, subject)
            .await?
            .map(|facts| facts.number)
            .unwrap_or_default();
        let key = repick_key(recall_id, subject.subject_id);
        let mut ticket = OpenTicket::new(
            subject.clone(),
            TASK_CATEGORY,
            &english(
                "manufacturing.recall.repick_title",
                &[("dispatch", &number)],
            ),
        )
        .description(&english(
            "manufacturing.recall.repick_body",
            &[("dispatch", &number), ("recall", &event.number)],
        ))
        .source_key(&key);
        if let Some(user) = storekeeper(state).await? {
            ticket = ticket.assign_to(Assignee::user(user));
        }
        let opened = task_creator(state).open(state, ticket).await?;
        let mut flag = RepickFlag {
            recall_id,
            dispatch_id: subject.subject_id,
            number,
            ticket_id: opened.map(|opened| opened.id.0),
            ..RepickFlag::default()
        };
        flag.save(&state.db).await?;
    }
    Ok(())
}

/// One line of the trace for the recall owner: a work order or a dispatch of
/// a customer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRow {
    /// The line of the recall, `inventory.recall_line:7`.
    pub line: Subject,
    /// The dispatch or the work order.
    pub subject: Subject,
    /// Its number, `DSP-2026-0003` or `WO-2026-0007`.
    pub label: String,
    /// The customer; `None` when the trace does not know.
    pub customer: Option<i64>,
    /// The recalled lot as printed on the pack, else its id.
    pub lot: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// `shipped` or `not_shipped`.
    pub shipped: bool,
    /// The day it was used (Asia/Dubai).
    pub on: Option<NaiveDate>,
    /// `pending`, `sent` or `waived`.
    pub notice: String,
    /// Whether it is a dispatch that left and has not been delivered: the
    /// recall owner brings it back.
    pub bring_back: bool,
    /// Whether it is a dispatch flagged for a re-pick.
    pub repick: bool,
    /// The task that asks for the re-pick, when there is one.
    pub task: Option<i64>,
}

/// The lines of recall `recall` that are dispatches or work orders, customer
/// first, then the oldest.
///
/// # Errors
///
/// The database's and the trace's errors.
pub async fn trace(state: &AppState, recall: i64) -> Result<Vec<TraceRow>> {
    let recall_subject = Subject::new(RECALL_SUBJECT_TYPE, recall);
    let holders = lot_trace(state).holders(state, &recall_subject).await?;
    let mut rows = Vec::new();
    for holder in holders {
        let Some(subject) = holder.subject.clone().filter(|subject| {
            subject.subject_type == DISPATCH_SUBJECT_TYPE || subject.subject_type == WORK_ORDER
        }) else {
            continue;
        };
        let mut row = TraceRow {
            line: holder.line.clone(),
            subject: subject.clone(),
            label: String::new(),
            customer: holder.party.map(|party| party.0),
            lot: String::new(),
            quantity_milli: holder.quantity_milli,
            shipped: holder.delivery_status.as_deref() != Some(NOT_SHIPPED),
            on: holder.on,
            notice: holder.notice.clone(),
            bring_back: false,
            repick: false,
            task: None,
        };
        if let Some(lot) = &holder.lot {
            row.lot = match lot_disposal(state).facts(state, lot).await? {
                Some(facts) => facts.number,
                None => lot.subject_id.to_string(),
            };
        }
        if subject.subject_type == DISPATCH_SUBJECT_TYPE {
            if let Some(facts) = dispatch_directory(state).facts(state, &subject).await? {
                row.label = facts.number;
                row.bring_back = facts.status == "dispatched";
            }
            if let Some(flag) = RepickFlag::of(&state.db, recall, subject.subject_id).await? {
                row.repick = true;
                row.task = flag.ticket_id;
            }
        } else if let Some(order) = WorkOrder::find(&state.db, subject.subject_id).await? {
            row.label = order.number;
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| {
        (a.customer, a.on, a.subject.subject_id).cmp(&(b.customer, b.on, b.subject.subject_id))
    });
    Ok(rows)
}

/// Registers the listener.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: RecallTraced, state: AppState| async move { on_traced(&state, &event).await },
    );
}
