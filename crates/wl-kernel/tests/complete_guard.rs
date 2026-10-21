//! The kernel's seam on `complete` (wl-scheduling §4.1; 05 `STK.LOT.01`):
//! with no vertical providing a guard every completion is allowed
//! ([`NoCompleteGuard`]); a provided guard is the one the app asks.

use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::scheduling::{
    BookingId, CheckInAnswer, CompleteGuard, NoCompleteGuard, SchedulingFuture, complete_guard,
    provide_complete_guard,
};

/// Refuses every completion with a text.
struct Refusing;

impl CompleteGuard for Refusing {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _booking: BookingId,
        _at: DateTime,
    ) -> SchedulingFuture<'a, CheckInAnswer> {
        Box::pin(async { Ok(CheckInAnswer::Refused("Not yet".to_owned())) })
    }
}

/// Without a provider the app's guard allows, and so does the default one.
#[renox::test]
async fn the_default_guard_allows_every_completion() {
    let app = TestApp::new(App::new()).await;
    let at = renox::db::now();
    assert_eq!(
        NoCompleteGuard
            .check(app.state(), BookingId(1), at)
            .await
            .unwrap(),
        CheckInAnswer::Allowed
    );
    assert_eq!(
        complete_guard(app.state())
            .check(app.state(), BookingId(7), at)
            .await
            .unwrap(),
        CheckInAnswer::Allowed
    );
}

/// A guard a vertical provides replaces the default.
#[renox::test]
async fn a_provided_guard_is_the_one_asked() {
    let app = TestApp::new(App::new().module(Providing)).await;
    assert_eq!(
        complete_guard(app.state())
            .check(app.state(), BookingId(7), renox::db::now())
            .await
            .unwrap(),
        CheckInAnswer::Refused("Not yet".to_owned())
    );
}

/// A module that provides the refusing guard.
struct Providing;

impl Module for Providing {
    fn name(&self) -> &'static str {
        "providing"
    }

    fn register(&self, app: &mut Registry) {
        provide_complete_guard(app, Refusing);
    }
}
