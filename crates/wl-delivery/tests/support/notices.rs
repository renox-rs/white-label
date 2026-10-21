//! A notifier for the delivery tests: the crate does not depend on
//! `wl-notify`, so the tests provide a `Notifier` that records the requests it
//! is asked for.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest};

/// The recording `Notifier`.
#[derive(Clone, Default)]
pub struct Notices(Arc<Mutex<Vec<NotifyRequest>>>);

impl Notices {
    /// Every request so far.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The requests of type `key`.
    pub fn of_type(&self, key: &str) -> Vec<NotifyRequest> {
        self.all()
            .into_iter()
            .filter(|r| r.type_key == key)
            .collect()
    }

    /// The users the requests of type `key` went to.
    pub fn users_of_type(&self, key: &str) -> Vec<i64> {
        self.of_type(key)
            .into_iter()
            .flat_map(|r| r.recipients)
            .filter_map(|r| match r {
                NotifyRecipient::User { id } => Some(id),
                _ => None,
            })
            .collect()
    }
}

impl Notifier for Notices {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}
