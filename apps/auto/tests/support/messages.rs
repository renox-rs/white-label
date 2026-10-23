//! The stand-in of the notifier (wl-notify): the notifications asked of it,
//! kept in memory so a test can read them back. It can be made to fail.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::notify::{
    MessageId, NotificationTypeDef, Notifier, NotifyChannel, NotifyFuture, NotifyRecipient,
    NotifyRequest,
};

/// The notifications asked of the notifier.
#[derive(Clone, Default)]
pub struct Messages(Arc<Mutex<Vec<NotifyRequest>>>, Arc<Mutex<bool>>);

impl Messages {
    /// Makes every further notification fail (or work again).
    pub fn fail(&self, failing: bool) {
        *self.1.lock().unwrap() = failing;
    }

    /// The notifications sent so far, oldest first.
    pub fn sent(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }

    /// The notifications of `type_key` sent so far.
    pub fn of_type(&self, type_key: &str) -> Vec<NotifyRequest> {
        self.sent()
            .into_iter()
            .filter(|n| n.type_key == type_key)
            .collect()
    }

    /// The English text of `request` for `channel`, subject and body joined:
    /// what a person reads.
    pub fn text(request: &NotifyRequest, channel: NotifyChannel) -> String {
        let def: Arc<NotificationTypeDef> = wl_kernel::notify::notification_type(&request.type_key)
            .expect("the type is registered");
        let template = def
            .template_for(channel, "en")
            .expect("an English text for the channel");
        let subject = template
            .subject
            .as_deref()
            .map(|s| wl_kernel::notify::fill(s, &request.data))
            .unwrap_or_default();
        format!(
            "{subject} {}",
            wl_kernel::notify::fill(&template.body, &request.data)
        )
    }

    /// Whether a notification went to the user `id`.
    pub fn told_user(request: &NotifyRequest, id: i64) -> bool {
        request
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::User { id: user } if *user == id))
    }

    /// Whether a notification went to the party `id`.
    pub fn told_party(request: &NotifyRequest, id: i64) -> bool {
        request
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::Party { id: party } if party.0 == id))
    }
}

impl Notifier for Messages {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        if *self.1.lock().unwrap() {
            return Box::pin(async { Err(Error::permanent_message("the notifier is down")) });
        }
        let mut sent = self.0.lock().unwrap();
        sent.push(request);
        let id = i64::try_from(sent.len()).unwrap();
        Box::pin(async move { Ok(vec![MessageId(id)]) })
    }
}
