//! The tickets the centre opens through the kernel's task creator, kept in
//! memory so a test can read them back: opened, commented, closed or cancelled.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::{
    OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef,
};

/// A ticket opened through the task creator.
#[derive(Debug, Clone)]
pub struct Opened {
    /// What it was asked for.
    pub ticket: OpenTicket,
    /// The id the stand-in gave it.
    pub id: i64,
    /// The comments added to it, oldest first.
    pub comments: Vec<String>,
    /// The note it was completed with, once closed.
    pub completed: Option<String>,
    /// The reason it was cancelled with, once cancelled.
    pub cancelled: Option<String>,
}

impl Opened {
    /// Whether it is still open.
    pub fn is_open(&self) -> bool {
        self.completed.is_none() && self.cancelled.is_none()
    }

    /// The user it is assigned to, if it is assigned to a user.
    pub fn assignee_user(&self) -> Option<i64> {
        match self.ticket.assignee {
            Some(wl_kernel::tickets::Assignee::User { id }) => Some(id),
            _ => None,
        }
    }
}

/// The tickets other modules open.
#[derive(Clone, Default)]
pub struct Tasks(Arc<Mutex<Vec<Opened>>>);

impl Tasks {
    /// Every ticket opened, oldest first.
    pub fn all(&self) -> Vec<Opened> {
        self.0.lock().unwrap().clone()
    }

    /// The tickets about `subject`, oldest first.
    pub fn about(&self, subject: &Subject) -> Vec<Opened> {
        self.all()
            .into_iter()
            .filter(|opened| opened.ticket.subject == *subject)
            .collect()
    }

    /// The tickets whose source key starts with `prefix`.
    pub fn with_key(&self, prefix: &str) -> Vec<Opened> {
        self.all()
            .into_iter()
            .filter(|opened| {
                opened
                    .ticket
                    .source_key
                    .as_deref()
                    .is_some_and(|key| key.starts_with(prefix))
            })
            .collect()
    }

    /// The tickets still open.
    pub fn open(&self) -> Vec<Opened> {
        self.all().into_iter().filter(Opened::is_open).collect()
    }
}

fn reference(id: i64) -> TicketRef {
    TicketRef {
        id: TicketId(id),
        number: format!("TKT-2026-{id:04}"),
    }
}

impl TaskCreator for Tasks {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let mut all = self.0.lock().unwrap();
        let id = i64::try_from(all.len()).unwrap() + 1;
        all.push(Opened {
            ticket,
            id,
            comments: Vec::new(),
            completed: None,
            cancelled: None,
        });
        Box::pin(async move { Ok(Some(reference(id))) })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|opened| opened.is_open() && opened.ticket.source_key.as_deref() == Some(source_key))
            .map(|opened| reference(opened.id));
        Box::pin(async move { Ok(found) })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        let mut all = self.0.lock().unwrap();
        let closed = match all
            .iter_mut()
            .find(|opened| opened.id == ticket.0 && opened.is_open())
        {
            Some(opened) => {
                opened.completed = Some(note.to_owned());
                true
            }
            None => false,
        };
        Box::pin(async move { Ok(closed) })
    }

    fn cancel_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        reason: &'a str,
    ) -> TicketFuture<'a, bool> {
        let mut all = self.0.lock().unwrap();
        let closed = match all
            .iter_mut()
            .find(|opened| opened.id == ticket.0 && opened.is_open())
        {
            Some(opened) => {
                opened.cancelled = Some(reason.to_owned());
                true
            }
            None => false,
        };
        Box::pin(async move { Ok(closed) })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        if let Some(opened) = self
            .0
            .lock()
            .unwrap()
            .iter_mut()
            .find(|opened| opened.id == ticket.0)
        {
            opened.comments.push(comment.text);
        }
        Box::pin(async { Ok(()) })
    }
}
