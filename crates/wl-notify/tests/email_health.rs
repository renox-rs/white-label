//! The e-mail provider reports to the integration health (wl-notify §4.1
//! `send`, wl-kernel integrations): a transient failure degrades the
//! connection the app registered under `EMAIL`, an accepted send brings it
//! back `up`, and a wrong address says nothing about it. Its own test file:
//! the registry of connections is process wide.

mod support;

use std::time::Duration;

use wl_kernel::integrations::EMAIL;
use wl_kernel::{ConnectionStatus, Delivery, IntegrationConnection, IntegrationDef};
use wl_notify::SendError;
use wl_notify::channels::HardError;

use support::{BLUE_DUNES, RECEIPT, RED_ROCK, REMINDER, boot};

fn register() {
    IntegrationDef::of(EMAIL, "notify", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .fallback("Notices stay visible in the portal.", None)
    .register();
}

async fn status(t: &support::Booted) -> Option<ConnectionStatus> {
    IntegrationConnection::by_key(t.db(), EMAIL)
        .await
        .unwrap()
        .map(|connection| connection.status)
}

#[renox::test]
async fn a_transient_failure_degrades_the_connection_and_a_sent_message_recovers_it() {
    register();
    let t = boot().await;
    t.email
        .then([Err(SendError::transient("421", "try again later"))]);
    t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(status(&t).await, Some(ConnectionStatus::Degraded));

    // The retry after a minute is accepted.
    t.after(Duration::from_secs(60)).await;
    assert_eq!(status(&t).await, Some(ConnectionStatus::Up));
}

#[renox::test]
async fn a_wrong_address_does_not_touch_the_connection() {
    register();
    let t = boot().await;
    t.email.then([Err(SendError::hard(
        HardError::HardBounce,
        "550",
        "no such user",
    ))]);
    t.notify_party(RECEIPT, RED_ROCK).await;
    t.run_jobs().await;
    assert_ne!(status(&t).await, Some(ConnectionStatus::Degraded));
    assert_ne!(status(&t).await, Some(ConnectionStatus::Down));
}
