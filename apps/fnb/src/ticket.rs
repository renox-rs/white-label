//! The kitchen ticket (06 F&B §7.1 *Kitchen ticket*; base for BL-FNB-04 to
//! BL-FNB-06): one ticket per item line of an accepted order, on the screen
//! of its station. Starting the first ticket of an order starts the order;
//! bumping the last one makes it `ready` ([`crate::kitchen`]).
//!
//! The base builds `start`, `bump` and the system's `void`; `recall` and the
//! station split of a line come with the kitchen display issue.

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::permissions;

/// The subject type of a ticket.
pub const SUBJECT_TYPE: &str = "fnb.ticket";

/// The station of a ticket until the menu says another.
pub const DEFAULT_STATION: &str = "pass";

/// `start`: queued → in_prep.
pub const START: &str = "start";
/// `bump`: queued, in_prep → done.
pub const BUMP: &str = "bump";
/// `void`: queued, in_prep → voided, by the system when the order is
/// cancelled.
pub const VOID: &str = "void";

/// Where a kitchen ticket is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TicketStatus {
    /// On the station's screen, not started.
    #[default]
    Queued,
    /// Being cooked; the timer runs.
    InPrep,
    /// Bumped by the station (terminal).
    Done,
    /// The order was cancelled before completion (terminal).
    Voided,
}

impl TicketStatus {
    /// Whether the kitchen started the line: it is being cooked or is done.
    pub fn is_started(self) -> bool {
        matches!(self, TicketStatus::InPrep | TicketStatus::Done)
    }
}

/// A kitchen ticket.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_tickets", state_machine, index(order_id), index(status))]
pub struct KitchenTicket {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "fnb_orders")]
    pub order_id: i64,
    /// The sale line (`wl-sales`) the ticket cooks.
    pub sale_line_id: i64,
    /// The station whose screen shows it.
    pub station: String,
    /// What to cook.
    pub description: String,
    /// How many, in thousandths.
    pub quantity_milli: i64,
    /// When it was started.
    pub started_at: Option<DateTime>,
    /// When it was bumped.
    pub bumped_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: TicketStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl KitchenTicket {
    /// The ticket as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The tickets of the order `order_id`, in order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_order(db: &Db, order_id: i64) -> Result<Vec<KitchenTicket>> {
        KitchenTicket::query()
            .where_(KitchenTicket::ORDER_ID.eq(order_id))
            .order_by(KitchenTicket::ID)
            .get(db)
            .await
    }

    /// The tickets still on a screen (queued or in preparation), oldest
    /// first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn open(db: &Db) -> Result<Vec<KitchenTicket>> {
        KitchenTicket::query()
            .where_(KitchenTicket::STATUS.is_in([TicketStatus::Queued, TicketStatus::InPrep]))
            .order_by(KitchenTicket::ID)
            .get(db)
            .await
    }
}

fn start() -> Transition<KitchenTicket> {
    Transition::new(START)
        .label("Start")
        .tone("primary")
        .permission(permissions::KDS_WORK)
        .from([TicketStatus::Queued])
        .to(TicketStatus::InPrep)
        .effect(|ticket: &KitchenTicket, _ctx, tx| {
            let id = ticket.id;
            Box::pin(async move {
                let at = Some(renox::db::now());
                KitchenTicket::query()
                    .where_(KitchenTicket::ID.eq(id))
                    .update(&mut *tx, &[("started_at", &at)])
                    .await?;
                Ok(())
            })
        })
}

fn bump() -> Transition<KitchenTicket> {
    Transition::new(BUMP)
        .label("Bump")
        .tone("success")
        .permission(permissions::KDS_WORK)
        .from([TicketStatus::Queued, TicketStatus::InPrep])
        .to(TicketStatus::Done)
        .effect(|ticket: &KitchenTicket, _ctx, tx| {
            let id = ticket.id;
            Box::pin(async move {
                let at = Some(renox::db::now());
                KitchenTicket::query()
                    .where_(KitchenTicket::ID.eq(id))
                    .update(&mut *tx, &[("bumped_at", &at)])
                    .await?;
                Ok(())
            })
        })
}

fn void() -> Transition<KitchenTicket> {
    Transition::new(VOID)
        .label("Void")
        .tone("danger")
        .system()
        .from([TicketStatus::Queued, TicketStatus::InPrep])
        .to(TicketStatus::Voided)
}

impl StateMachine for KitchenTicket {
    type State = TicketStatus;
    const COLUMN: renox::db::Col<KitchenTicket, TicketStatus> = KitchenTicket::STATUS;

    fn machine() -> Machine<Self> {
        use TicketStatus::{Done, InPrep, Queued, Voided};
        Machine::new()
            .initial([Queued])
            .state(Queued)
            .tone("neutral")
            .state(InPrep)
            .tone("info")
            .state(Done)
            .tone("success")
            .terminal()
            .state(Voided)
            .tone("neutral")
            .terminal()
            .transition(start())
            .transition(bump())
            .transition(void())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_declares_the_four_states_and_the_three_transitions_of_the_base() {
        let machine = KitchenTicket::machine();
        assert_eq!(machine.states().len(), 4);
        for name in [START, BUMP, VOID] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        assert!(machine.find(VOID).unwrap().is_system());
        assert!(!machine.find(START).unwrap().is_system());
    }

    #[test]
    fn a_line_has_started_once_its_ticket_is_cooking_or_done() {
        assert!(!TicketStatus::Queued.is_started());
        assert!(TicketStatus::InPrep.is_started());
        assert!(TicketStatus::Done.is_started());
        assert!(!TicketStatus::Voided.is_started());
    }
}
