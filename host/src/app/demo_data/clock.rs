//! The seed's clock. The demo has 15 months of history, and the modules read
//! the time (`renox::db::now()`, `today()`) for what they stamp and what they
//! check: the day a document was accepted, the dispute window of a deposit,
//! the day an invoice is issued. So a step of the story runs *at* its day.
//!
//! Renox moves the clock of a task only through `TestApp::travel`, which adds
//! seconds and never takes them away. [`Clock`] is a `TestApp` of an empty
//! app used for that alone: it never serves, and its database is a throwaway
//! in memory. Going back to a day in the past is an addition that wraps: the
//! offset is an `AtomicI64`, whose `fetch_add` wraps around on overflow, so
//! adding `i64::MAX` and then `2^63 + 1 - n` leaves the offset at `-n`.
//! Everything the step starts runs on the moved clock (`TestApp::
//! at_travelled_time`), the requests to the app's router included.

use std::future::Future;

use renox::chrono::{NaiveDate, NaiveDateTime};
use renox::testing::TestApp;
use wl_kernel::BusinessCalendar;

/// A clock that can be set to any past moment for the length of a step.
pub struct Clock {
    app: TestApp,
}

impl Clock {
    /// A clock on the real time.
    pub async fn new() -> Self {
        Self {
            app: TestApp::new(renox::App::new()).await,
        }
    }

    /// Puts the clock `seconds` from the real time (negative: before).
    fn set(&self, seconds: i64) {
        self.app.travel_back();
        if seconds >= 0 {
            self.app
                .travel(std::time::Duration::from_secs(seconds as u64));
        } else {
            let back = seconds.unsigned_abs();
            // `i64::MAX + (2^63 + 1 - back)` wraps to `-back`.
            self.app
                .travel(std::time::Duration::from_secs(i64::MAX as u64));
            self.app
                .travel(std::time::Duration::from_secs((1u64 << 63) + 1 - back));
        }
    }

    /// Runs `step` as if it were `at` (Asia/Dubai wall-clock time); the real
    /// time again afterwards. A moment in the future is the real time.
    pub async fn at_time<F: Future>(&self, at: NaiveDateTime, step: F) -> F::Output {
        let wanted = BusinessCalendar::default().from_local(at);
        let seconds = (wanted - renox::db::now()).num_seconds().min(0);
        self.set(seconds);
        let out = self.app.at_travelled_time(step).await;
        self.set(0);
        out
    }

    /// Runs `step` at 10:00 on `day` (Asia/Dubai).
    pub async fn at<F: Future>(&self, day: NaiveDate, step: F) -> F::Output {
        let at = day.and_hms_opt(10, 0, 0).unwrap_or_default();
        self.at_time(at, step).await
    }
}
