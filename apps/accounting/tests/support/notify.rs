//! The real wl-notify in the accounting tests: a scripted e-mail provider.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::notify::NotifyChannel;
use wl_notify::channels::ProviderFuture;
use wl_notify::{ChannelProvider, Outgoing, SendError};

use super::Booted;
use wl_parties::{ContactPoint, ContactType};

/// An e-mail provider that sends every message unless a test scripted the
/// next replies ([`ScriptedMail::then`]); it keeps what it was given.
#[derive(Clone, Default)]
pub struct ScriptedMail {
    replies: Arc<Mutex<VecDeque<Result<String, SendError>>>>,
    sent: Arc<Mutex<Vec<Outgoing>>>,
}

impl ScriptedMail {
    /// The next sends answer with `replies`, in order.
    pub fn then(&self, replies: impl IntoIterator<Item = Result<String, SendError>>) {
        self.replies.lock().unwrap().extend(replies);
    }

    /// Every message it was asked to send, successful or not.
    pub fn attempts(&self) -> Vec<Outgoing> {
        self.sent.lock().unwrap().clone()
    }
}

impl ChannelProvider for ScriptedMail {
    fn name(&self) -> &'static str {
        "scripted-email"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, Result<String, SendError>> {
        self.sent.lock().unwrap().push(outgoing.clone());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("accepted".to_owned()));
        Box::pin(async move { reply })
    }
}

impl Booted {
    /// Gives `party` a primary, verified e-mail contact point `address`.
    pub async fn email_for(&self, party: wl_kernel::PartyId, address: &str) -> ContactPoint {
        let mut point = ContactPoint {
            party_id: party.0,
            contact_type: ContactType::Email,
            value: address.to_owned(),
            is_primary: true,
            verified_at: Some(renox::db::now()),
            ..ContactPoint::default()
        };
        point
            .insert(self.db())
            .await
            .expect("the contact point is stored");
        point
    }
}
