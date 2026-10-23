//! The stand-in of the ticket book (wl-ticketing): the warranty tickets of a
//! sale or job card and where they stand. It keeps what it is told in memory.

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::{
    OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, TicketSummary,
};

/// The tickets the workshop has, by the record they are about and their
/// category.
#[derive(Clone, Default)]
pub struct Tickets {
    all: Arc<Mutex<Vec<(Subject, String, TicketSummary)>>>,
}

impl Tickets {
    /// A ticket of `category` about `about`: `id`, `number`, `status`.
    pub fn add(&self, about: &Subject, category: &str, id: i64, number: &str, status: &str) {
        self.all.lock().unwrap().push((
            about.clone(),
            category.to_owned(),
            TicketSummary {
                id: TicketId(id),
                number: number.to_owned(),
                title: "Failed part".to_owned(),
                status: status.to_owned(),
            },
        ));
    }
}

impl TaskCreator for Tickets {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        _ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async { Ok(None) })
    }

    fn tickets_of<'a>(
        &'a self,
        _state: &'a AppState,
        subject: &'a Subject,
        category_key: &'a str,
    ) -> TicketFuture<'a, Vec<TicketSummary>> {
        let found: Vec<TicketSummary> = self
            .all
            .lock()
            .unwrap()
            .iter()
            .filter(|(about, category, _)| about == subject && category == category_key)
            .map(|(_, _, ticket)| ticket.clone())
            .collect();
        Box::pin(async move { Ok(found) })
    }
}
