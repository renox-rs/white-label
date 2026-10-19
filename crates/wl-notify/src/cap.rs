//! The marketing frequency cap (wl-notify §6 `NTF.RUL.02`, #62 A7, A8).
//!
//! A party gets at most [`CapSettings::max`] marketing messages in 7 days
//! (default 2), counted over every channel and every broadcast. A message
//! counts once the provider accepted it (`sent_at` is set); `suppressed`,
//! `abandoned`, `deferred` and `queued` ones never do. Only messages to a
//! party are capped, and only of a type with category `marketing`.
//!
//! The count and the provider send of a capped message run under one lock
//! per party ([`lock`]), so two sends for the same party cannot both pass the
//! cap.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use renox::chrono::TimeDelta;
use renox::db::DateTime;
use renox::prelude::*;
use renox::tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use wl_kernel::notify::Category;

use crate::message::Message;

/// The marketing messages a party may get in [`WINDOW_DAYS`], unless set.
pub const DEFAULT_MAX: i64 = 2;

/// The days the cap looks back (7 x 24 hours).
pub const WINDOW_DAYS: i64 = 7;

/// The module's cap setting, set on [`WlNotify`](crate::WlNotify) with
/// `marketing_frequency_cap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapSettings {
    /// A party that already got this many marketing messages in 7 days gets
    /// no more.
    pub max: i64,
}

impl Default for CapSettings {
    fn default() -> Self {
        CapSettings { max: DEFAULT_MAX }
    }
}

impl CapSettings {
    /// Whether a party with `recent` marketing messages in 7 days is at the cap.
    pub fn reached(&self, recent: i64) -> bool {
        recent >= self.max
    }
}

/// The app's cap setting, or the default when none is provided.
pub fn settings(state: &AppState) -> CapSettings {
    state
        .provided::<CapSettings>()
        .map(|s| *s)
        .unwrap_or_default()
}

/// The start of the cap's look-back as of `now`: 7 x 24 hours earlier.
pub fn since(now: DateTime) -> DateTime {
    now - TimeDelta::days(WINDOW_DAYS)
}

/// Whether the type `key` has category `marketing`. A type nobody knows any
/// more is not.
async fn is_marketing(state: &AppState, key: &str) -> bool {
    crate::notifier::type_def(&state.db, key)
        .await
        .is_ok_and(|def| def.category == Category::Marketing)
}

/// How many marketing messages party `party` had accepted by a provider at
/// `since` or later, on any channel and in any broadcast.
pub async fn count_recent(state: &AppState, party: i64, since: DateTime) -> Result<i64> {
    let sent = Message::query()
        .where_(Message::PARTY_ID.eq(party))
        .where_(Message::SENT_AT.gte(since))
        .get(&state.db)
        .await?;
    let keys: BTreeSet<&str> = sent.iter().map(|m| m.type_key.as_str()).collect();
    let mut marketing = BTreeSet::new();
    for key in keys {
        if is_marketing(state, key).await {
            marketing.insert(key);
        }
    }
    Ok(sent
        .iter()
        .filter(|m| marketing.contains(m.type_key.as_str()))
        .count() as i64)
}

/// Whether `message` is subject to the cap: a marketing message to a party.
pub async fn applies(state: &AppState, message: &Message) -> Result<bool> {
    if message.party_id.is_none() {
        return Ok(false);
    }
    let def = crate::notifier::type_def(&state.db, &message.type_key).await?;
    Ok(def.category == Category::Marketing)
}

/// Whether `message`'s party is at the cap as of `now`. Call it under
/// [`lock`].
pub async fn reached(state: &AppState, message: &Message, now: DateTime) -> Result<bool> {
    let Some(party) = message.party_id else {
        return Ok(false);
    };
    let recent = count_recent(state, party, since(now)).await?;
    Ok(settings(state).reached(recent))
}

type Locks = Mutex<HashMap<i64, Arc<AsyncMutex<()>>>>;

fn locks() -> &'static Locks {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    LOCKS.get_or_init(Mutex::default)
}

/// The lock of one party, released when dropped.
#[derive(Debug)]
pub struct PartyLock(#[allow(dead_code)] OwnedMutexGuard<()>);

/// Takes the lock of party `party`, waiting for any send that holds it.
/// Locks nobody holds or waits for are dropped from the table on the way.
pub async fn lock(party: i64) -> PartyLock {
    let mutex = {
        let mut table = locks().lock().unwrap_or_else(|e| e.into_inner());
        table.retain(|_, m| Arc::strong_count(m) > 1);
        Arc::clone(table.entry(party).or_default())
    };
    PartyLock(mutex.lock_owned().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_cap_is_two_in_seven_days() {
        let cap = CapSettings::default();
        assert_eq!(cap.max, 2);
        assert!(!cap.reached(1));
        assert!(cap.reached(2));
        assert!(cap.reached(3));
        assert_eq!(WINDOW_DAYS, 7);
    }

    #[test]
    fn the_look_back_is_seven_times_twenty_four_hours() {
        let now = renox::db::now();
        assert_eq!(now - since(now), TimeDelta::hours(7 * 24));
    }

    #[test]
    fn the_reason_is_stable() {
        assert_eq!(crate::message::FREQUENCY_CAP, "frequency_cap");
    }
}
