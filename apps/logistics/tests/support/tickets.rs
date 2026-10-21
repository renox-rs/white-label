//! A ticket desk for the logistics tests: the crate does not depend on
//! `wl-ticketing`, so the tests provide a `TaskCreator` that records the
//! tickets it is asked to open, resolve and comment on, and answers `TKT-n`;
//! switched off, it opens nothing (as without the ticketing module).

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::tickets::{OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef};

/// The recording `TaskCreator`.
#[derive(Clone, Default)]
pub struct TicketDesk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    resolved: Arc<Mutex<Vec<(TicketId, String)>>>,
    comments: Arc<Mutex<Vec<(TicketId, TaskComment)>>>,
    completed: Arc<Mutex<Vec<(TicketId, String)>>>,
    off: Arc<AtomicBool>,
}

impl TicketDesk {
    /// The tickets opened, oldest first (ticket `n` is the n-th).
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// The tickets resolved by their source, with the note.
    pub fn resolved(&self) -> Vec<(TicketId, String)> {
        self.resolved.lock().unwrap().clone()
    }

    /// The comments added, with their ticket.
    pub fn comments(&self) -> Vec<(TicketId, TaskComment)> {
        self.comments.lock().unwrap().clone()
    }

    /// The tickets closed by their source, with the note.
    pub fn completed(&self) -> Vec<(TicketId, String)> {
        self.completed.lock().unwrap().clone()
    }

    /// Switches ticketing off (`true`) or on again.
    pub fn off(&self, off: bool) {
        self.off.store(off, Ordering::SeqCst);
    }
}

impl TaskCreator for TicketDesk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            if self.off.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn resolve_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.resolved
                .lock()
                .unwrap()
                .push((ticket, note.to_owned()));
            Ok(true)
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.completed
                .lock()
                .unwrap()
                .push((ticket, note.to_owned()));
            Ok(true)
        })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async move {
            self.comments.lock().unwrap().push((ticket, comment));
            Ok(())
        })
    }
}
