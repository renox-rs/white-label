//! The rate card contract (wl-invoicing §6 `RateCards`, `FIN.RAT.01`): "has
//! this contract an accepted rate card, and does it price this role on this
//! shift?". 09 Manpower asks it when a client contract is activated (a rate
//! card version accepted by the client, `DOC.SIG.01`) and when a manpower
//! request is submitted (the rate card covers the role and the shift),
//! without depending on the `wl-invoicing` crate.
//!
//! The rate cards themselves (§3 *Rate card*, the versions of `MPW.ACL.16`
//! and the pricing of `FIN.USG.01`) are built in `wl-invoicing`, which
//! provides this seam ([`provide_rate_cards`]) when its module is on. Until a
//! module provides it, [`rate_cards`] gives [`NoRateCards`], which knows no
//! card: a guard that cannot find the rate card does not let the work
//! through.
//!
//! ```ignore
//! use wl_kernel::rate_cards::rate_cards;
//!
//! let cards = rate_cards(&state);
//! let source = Subject::new("manpower.client_contract", 4);
//! if cards.accepted_version(&state, &source, today).await?.is_none() { /* refuse */ }
//! if !cards.covers(&state, &source, "guard", "day", today).await? { /* refuse */ }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// An accepted version of a rate card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateCardVersion {
    /// The rate card.
    pub rate_card: i64,
    /// The version number, from 1.
    pub version: i64,
    /// The first day the version prices.
    pub effective_from: NaiveDate,
}

/// The future a [`RateCards`] method returns.
pub type RateCardFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The rate cards of wl-invoicing. `wl-invoicing` provides it; without it,
/// [`rate_cards`] gives [`NoRateCards`].
pub trait RateCards: Send + Sync + 'static {
    /// The version of the rate card whose source contract is `source` that
    /// the client accepted and that is in force on `on` (the latest accepted
    /// version effective on or before that day). A *pending* version never
    /// counts. The default knows none.
    fn accepted_version<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
        _on: NaiveDate,
    ) -> RateCardFuture<'a, Option<RateCardVersion>> {
        Box::pin(async { Ok(None) })
    }

    /// Whether the accepted version in force on `on` has a line for `role`
    /// on the shift type `shift`. The default is no.
    fn covers<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
        _role: &'a str,
        _shift: &'a str,
        _on: NaiveDate,
    ) -> RateCardFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The rate cards when no module provides them: no contract has a card.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRateCards;

impl RateCards for NoRateCards {}

/// Makes `cards` the app's [`RateCards`]; `wl-invoicing` calls it in
/// `register`.
pub fn provide_rate_cards(app: &mut Registry, cards: impl RateCards) {
    let cards: Arc<dyn RateCards> = Arc::new(cards);
    app.provide(cards);
}

/// The app's [`RateCards`], or [`NoRateCards`] when none is provided.
pub fn rate_cards(state: &AppState) -> Arc<dyn RateCards> {
    match state.provided::<Arc<dyn RateCards>>() {
        Some(cards) => Arc::clone(&cards),
        None => Arc::new(NoRateCards),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_round_trips_through_json() {
        let version = RateCardVersion {
            rate_card: 3,
            version: 2,
            effective_from: NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
        };
        let json = renox::serde_json::to_string(&version).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<RateCardVersion>(&json).unwrap(),
            version
        );
    }
}
