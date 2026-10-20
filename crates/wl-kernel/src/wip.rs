//! Unbilled time, or work in progress (WIP), of an agreement: the contract a
//! vertical uses to ask how much time on its engagement is not billed yet,
//! and to write it off, without depending on the module that records time.
//!
//! Decision: time entries belong to `wl-attendance` (`HRM.TMS.01`), which is
//! not built. This is the seam it will provide ([`provide_unbilled_time`]);
//! until then [`unbilled_time`] gives [`NoUnbilledTime`], whose WIP is always
//! zero, so an engagement never waits on time nobody records. The vertical
//! bills WIP through the invoicing contract and writes it off through
//! [`UnbilledTime::write_off`].
//!
//! ```ignore
//! use wl_kernel::wip::unbilled_time;
//!
//! // 04 engagement `end`: is there unbilled time left?
//! let wip = unbilled_time(&state).summary(&state, &engagement.subject()).await?;
//! if wip.unbilled > 0 { /* the partner bills it or writes it off */ }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// What the time recorded on an agreement amounts to, in fils of the client's
/// billing currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WipSummary {
    /// Approved time not billed and not written off yet.
    pub unbilled: i64,
    /// Time billed on an invoice so far.
    pub billed: i64,
}

/// The future an [`UnbilledTime`] method returns.
pub type WipFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Reads and writes off the unbilled time of an agreement. The module that
/// records time provides it; without one, [`unbilled_time`] gives
/// [`NoUnbilledTime`].
pub trait UnbilledTime: Send + Sync + 'static {
    /// The WIP of `agreement`; the default has none.
    fn summary<'a>(
        &'a self,
        _state: &'a AppState,
        _agreement: &'a Subject,
    ) -> WipFuture<'a, WipSummary> {
        Box::pin(async { Ok(WipSummary::default()) })
    }

    /// Writes `amount` fils of the unbilled time of `agreement` off, with
    /// the user `by` who decided and the `reason`. The default does
    /// nothing.
    fn write_off<'a>(
        &'a self,
        _state: &'a AppState,
        _agreement: &'a Subject,
        _amount: i64,
        _reason: &'a str,
        _by: Option<i64>,
    ) -> WipFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// The unbilled time when no module records time: none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUnbilledTime;

impl UnbilledTime for NoUnbilledTime {}

/// Makes `time` the app's [`UnbilledTime`].
pub fn provide_unbilled_time(app: &mut Registry, time: impl UnbilledTime) {
    let time: Arc<dyn UnbilledTime> = Arc::new(time);
    app.provide(time);
}

/// The app's [`UnbilledTime`], or [`NoUnbilledTime`] when none is provided.
pub fn unbilled_time(state: &AppState) -> Arc<dyn UnbilledTime> {
    match state.provided::<Arc<dyn UnbilledTime>>() {
        Some(time) => Arc::clone(&time),
        None => Arc::new(NoUnbilledTime),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_unbilled_without_a_module() {
        assert_eq!(WipSummary::default().unbilled, 0);
        assert_eq!(WipSummary::default().billed, 0);
    }
}
