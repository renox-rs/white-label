//! The module's clock (wl-notify §7): where `send`, `release` and the window
//! guards take "now" from. The default is [`renox::db::now`]; a test or an app
//! provides a fixed one, as it provides a channel provider.

use std::fmt;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;

/// The current instant, replaceable. Cheap to clone.
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> DateTime + Send + Sync>);

impl Clock {
    /// The real clock: [`renox::db::now`].
    pub fn system() -> Self {
        Self(Arc::new(renox::db::now))
    }

    /// A clock that always says `at`.
    pub fn fixed(at: DateTime) -> Self {
        Self(Arc::new(move || at))
    }

    /// A clock that asks `now` every time.
    pub fn from_fn(now: impl Fn() -> DateTime + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    /// The current instant.
    pub fn now(&self) -> DateTime {
        (self.0)()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

impl fmt::Debug for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Clock").field(&self.now()).finish()
    }
}

/// The app's clock: the one given to `App::provide` (or the module's own),
/// else the real one.
pub fn clock(state: &AppState) -> Clock {
    match state.provided::<Clock>() {
        Some(clock) => (*clock).clone(),
        None => Clock::system(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_clock_says_the_same_instant() {
        let at = renox::db::now();
        let clock = Clock::fixed(at);
        assert_eq!(clock.now(), at);
        assert_eq!(clock.clone().now(), at);
    }

    #[test]
    fn the_system_clock_does_not_run_backwards() {
        let before = renox::db::now();
        assert!(Clock::default().now() >= before);
    }
}
