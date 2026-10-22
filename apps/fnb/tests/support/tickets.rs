//! A ticket desk for the F&B tests: the crate does not depend on
//! `wl-ticketing`, so the tests provide a `TaskCreator` that records the
//! tickets it is asked to open and answers `TKT-n`, finds an open one by its
//! source key, and records the ones it is asked to close.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::tickets::{OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef};

/// The recording `TaskCreator`.
#[derive(Clone, Default)]
pub struct TicketDesk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    closed: Arc<Mutex<Vec<i64>>>,
}

impl TicketDesk {
    /// The tickets opened, oldest first (ticket `n` is the n-th).
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }

    /// The ids of the tickets that were closed from their source.
    pub fn closed(&self) -> Vec<i64> {
        self.closed.lock().unwrap().clone()
    }

    /// The tickets opened with the source key `key`, oldest first.
    pub fn opened_with(&self, key: &str) -> Vec<OpenTicket> {
        self.opened()
            .into_iter()
            .filter(|ticket| ticket.source_key.as_deref() == Some(key))
            .collect()
    }
}

impl TaskCreator for TicketDesk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let opened = self.opened.lock().unwrap();
            let closed = self.closed.lock().unwrap();
            let found = opened
                .iter()
                .position(|ticket| ticket.source_key.as_deref() == Some(source_key))
                .map(|index| index as i64 + 1)
                .filter(|n| !closed.contains(n));
            Ok(found.map(|n| TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        _note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut closed = self.closed.lock().unwrap();
            if closed.contains(&ticket.0) {
                Ok(false)
            } else {
                closed.push(ticket.0);
                Ok(true)
            }
        })
    }
}
