//! The e-invoicing recipient lookup (wl-parties §3 *Company details*,
//! wl-invoicing §4): how a module finds the Peppol participant ID of a
//! company by its TRN through the ASP adapter of a legal entity, without
//! depending on the `wl-invoicing` crate.
//!
//! `wl-invoicing` provides a [`RecipientLookup`] with
//! [`provide_recipient_lookup`]. Other modules ask through [`lookup`], which
//! checks the health of the ASP connection first and reports the call to the
//! integration health of the kernel:
//!
//! ```ignore
//! use wl_kernel::einvoice::{self, Lookup};
//!
//! let entity = LegalEntity::for_scope(&state.db, None).await?;
//! match einvoice::lookup(&state, &entity, "100123456700003").await? {
//!     Lookup::Found(id) => { /* fill the field for the user to confirm */ }
//!     Lookup::NotFound | Lookup::Failed(_) | Lookup::Unavailable => { /* enter by hand */ }
//! }
//! ```
//!
//! Without a provider every lookup is [`Lookup::Unavailable`]: the field is
//! entered by hand.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;

use crate::entities::LegalEntity;
use crate::integrations::{self, ConnectionStatus, IntegrationConnection};

/// The future a [`RecipientLookup`] method returns.
pub type LookupFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What the ASP answered to a lookup by TRN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The recipient identifier (scheme + identifier), e.g.
    /// `0235:100123456700003`.
    Found(String),
    /// The ASP does not know the TRN.
    NotFound,
}

/// What [`lookup`] made of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// The identifier, trimmed, to fill in for the user to confirm.
    Found(String),
    /// The ASP does not know the TRN: "enter it by hand".
    NotFound,
    /// The call failed (the message is for logs, never for the user); the
    /// failure was reported to the integration health.
    Failed(String),
    /// The entity has no ASP adapter, there is no provider, or the ASP
    /// connection is not `up`: no call was made.
    Unavailable,
}

/// Looks up a recipient identifier through an ASP adapter. `wl-invoicing`
/// provides it; without that module, [`recipient_lookup`] gives
/// [`NoRecipientLookup`].
pub trait RecipientLookup: Send + Sync + 'static {
    /// The key of the integration connection that carries the lookups of
    /// `entity` (e.g. `asp.acme`); `None` when the entity has no ASP adapter.
    fn connection(&self, entity: &LegalEntity) -> Option<String>;

    /// Asks the ASP of `entity` for the identifier of the company with the
    /// 15-digit `trn`. An `Err` is a failed call.
    fn lookup<'a>(
        &'a self,
        state: &'a AppState,
        entity: &'a LegalEntity,
        trn: &'a str,
    ) -> LookupFuture<'a, Answer>;
}

/// The provider when the `invoicing` module is off: no entity has an ASP.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRecipientLookup;

impl RecipientLookup for NoRecipientLookup {
    fn connection(&self, _entity: &LegalEntity) -> Option<String> {
        None
    }

    fn lookup<'a>(
        &'a self,
        _state: &'a AppState,
        _entity: &'a LegalEntity,
        _trn: &'a str,
    ) -> LookupFuture<'a, Answer> {
        Box::pin(async { Ok(Answer::NotFound) })
    }
}

/// Makes `lookup` the app's [`RecipientLookup`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_recipient_lookup(app: &mut Registry, lookup: impl RecipientLookup) {
    let lookup: Arc<dyn RecipientLookup> = Arc::new(lookup);
    app.provide(lookup);
}

/// The app's [`RecipientLookup`], or [`NoRecipientLookup`] when none is
/// provided.
pub fn recipient_lookup(state: &AppState) -> Arc<dyn RecipientLookup> {
    match state.provided::<Arc<dyn RecipientLookup>>() {
        Some(lookup) => Arc::clone(&lookup),
        None => Arc::new(NoRecipientLookup),
    }
}

/// The ASP connection of `entity` when it is `up` and so can take a lookup;
/// `None` when the entity has no adapter, the connection is unknown, or it is
/// `degraded`, `down` or `disabled`.
pub async fn available_connection(
    state: &AppState,
    entity: &LegalEntity,
) -> Result<Option<IntegrationConnection>> {
    let Some(key) = recipient_lookup(state).connection(entity) else {
        return Ok(None);
    };
    let connection = IntegrationConnection::by_key(&state.db, &key).await?;
    Ok(connection.filter(|c| c.status == ConnectionStatus::Up))
}

/// Looks up the identifier of the company with `trn` through the ASP adapter
/// of `entity`. Nothing is called (and [`Lookup::Unavailable`] is the answer)
/// unless the connection is `up`; a call that fails or succeeds is reported
/// to the connection's health.
pub async fn lookup(state: &AppState, entity: &LegalEntity, trn: &str) -> Result<Lookup> {
    let Some(connection) = available_connection(state, entity).await? else {
        return Ok(Lookup::Unavailable);
    };
    let provider = recipient_lookup(state);
    match provider.lookup(state, entity, trn.trim()).await {
        Ok(answer) => {
            integrations::report_success(state, &connection.key).await?;
            Ok(match answer {
                Answer::Found(id) if !id.trim().is_empty() => Lookup::Found(id.trim().to_owned()),
                Answer::Found(_) | Answer::NotFound => Lookup::NotFound,
            })
        }
        Err(error) => {
            let message = error.to_string();
            integrations::report_failure(state, &connection.key, &message).await?;
            Ok(Lookup::Failed(message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_provides_a_lookup_until_a_module_does() {
        let entity = LegalEntity::default();
        assert_eq!(NoRecipientLookup.connection(&entity), None);
    }
}
