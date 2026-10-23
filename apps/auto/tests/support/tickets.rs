//! The stand-in of the ticket book (wl-ticketing): the warranty tickets of a
//! sale or job card and where they stand, and the tasks other modules open
//! through the task creator. It keeps what it is told in memory.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::{
    Assignee, OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef,
    TicketSummary,
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
            Some(Assignee::User { id }) => Some(id),
            _ => None,
        }
    }
}

/// The tickets the workshop has, by the record they are about and their
/// category, and the tasks opened.
#[derive(Clone, Default)]
pub struct Tickets {
    all: Arc<Mutex<Vec<(Subject, String, TicketSummary)>>>,
    opened: Arc<Mutex<Vec<Opened>>>,
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

    /// Every task opened, oldest first.
    pub fn tasks(&self) -> Vec<Opened> {
        self.opened.lock().unwrap().clone()
    }

    /// The tasks about `subject`, oldest first.
    pub fn about(&self, subject: &Subject) -> Vec<Opened> {
        self.tasks()
            .into_iter()
            .filter(|opened| opened.ticket.subject == *subject)
            .collect()
    }

    /// The tasks whose source key starts with `prefix`.
    pub fn with_key(&self, prefix: &str) -> Vec<Opened> {
        self.tasks()
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

    /// The tasks still open.
    pub fn open_tasks(&self) -> Vec<Opened> {
        self.tasks().into_iter().filter(Opened::is_open).collect()
    }
}

fn reference(id: i64) -> TicketRef {
    TicketRef {
        id: TicketId(id),
        number: format!("TKT-2026-{id:04}"),
    }
}

impl TaskCreator for Tickets {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        let mut all = self.opened.lock().unwrap();
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
            .opened
            .lock()
            .unwrap()
            .iter()
            .find(|opened| {
                opened.is_open() && opened.ticket.source_key.as_deref() == Some(source_key)
            })
            .map(|opened| reference(opened.id));
        Box::pin(async move { Ok(found) })
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

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        let mut all = self.opened.lock().unwrap();
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
        let mut all = self.opened.lock().unwrap();
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
            .opened
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
