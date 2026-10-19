//! The consent contract (wl-notify §4.3, `NTF.ACL.01`): whether a party agreed
//! to receive a kind of message on a channel, without `wl-notify` depending on
//! the module that records consents (`wl-parties`).
//!
//! That module provides a [`Consents`] with [`provide_consents`]; `wl-notify`
//! reads it through [`consents`]. Without a provider, [`NoConsents`] grants
//! nothing and records nothing, so with no consent module nobody receives
//! marketing.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;

use crate::notify::NotifyChannel;
use crate::parties::PartyId;

/// The purpose of marketing messages.
pub const MARKETING: &str = "marketing";

/// The future a [`Consents`] method returns.
pub type ConsentFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads and withdraws consents.
pub trait Consents: Send + Sync + 'static {
    /// Whether `party` has a consent for `purpose` (e.g. [`MARKETING`]) on
    /// `channel` that is given and not withdrawn.
    fn has_consent<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool>;

    /// Withdraws the consent of `party` for `purpose` on `channel`;
    /// `source` says how (e.g. `unsubscribe_link`, `sms_stop`). Returns
    /// `true` only when a consent was given and is now withdrawn, so a
    /// second call is a harmless `false`.
    fn withdraw<'a>(
        &'a self,
        db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        channel: NotifyChannel,
        source: &'a str,
    ) -> ConsentFuture<'a, bool>;
}

/// The consents when no module provides them: nobody has consented and
/// nothing can be withdrawn.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoConsents;

impl Consents for NoConsents {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _purpose: &'a str,
        _channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn withdraw<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _purpose: &'a str,
        _channel: NotifyChannel,
        _source: &'a str,
    ) -> ConsentFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// Makes `consents` the app's [`Consents`]; the module that records consents
/// calls it in `register`.
pub fn provide_consents(app: &mut Registry, consents: impl Consents) {
    let consents: Arc<dyn Consents> = Arc::new(consents);
    app.provide(consents);
}

/// The app's [`Consents`], or [`NoConsents`] when none is provided.
pub fn consents(state: &AppState) -> Arc<dyn Consents> {
    match state.provided::<Arc<dyn Consents>>() {
        Some(consents) => Arc::clone(&consents),
        None => Arc::new(NoConsents),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marketing_purpose_is_a_fixed_key() {
        assert_eq!(MARKETING, "marketing");
    }

    #[test]
    fn the_fallback_is_object_safe() {
        let fallback: Arc<dyn Consents> = Arc::new(NoConsents);
        drop(fallback);
    }
}
