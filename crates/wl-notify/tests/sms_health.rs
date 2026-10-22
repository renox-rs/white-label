//! The SMS provider reports to the integration health like the e-mail one
//! (wl-notify §4.1 `send`, wl-kernel integrations): a transient failure
//! degrades the connection the app registered under `SMS`, an accepted send
//! brings it back `up`. Its own test file: the registry of connections is
//! process wide.

mod support;

use std::time::Duration;

use wl_kernel::integrations::SMS;
use wl_kernel::{ConnectionStatus, Delivery, IntegrationConnection, IntegrationDef};
use wl_notify::SendError;

use support::{BLUE_DUNES, REMINDER, boot};

fn register() {
    IntegrationDef::of(SMS, "notify", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .fallback("The front desk calls the patients.", None)
    .register();
}

async fn status(t: &support::Booted) -> Option<ConnectionStatus> {
    IntegrationConnection::by_key(t.db(), SMS)
        .await
        .unwrap()
        .map(|connection| connection.status)
}

#[renox::test]
async fn a_transient_sms_failure_degrades_the_connection_and_a_sent_message_recovers_it() {
    register();
    let t = boot().await;
    // The reminder goes by e-mail first; the address refuses it for good, so
    // the SMS is tried, and the gateway is down for the first attempt.
    t.email
        .then([Err(SendError::permanent("550", "mailbox unknown"))]);
    t.sms
        .then([Err(SendError::transient("421", "try again later"))]);
    t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(status(&t).await, Some(ConnectionStatus::Degraded));

    // The retry after a minute is accepted.
    t.after(Duration::from_secs(60)).await;
    assert_eq!(status(&t).await, Some(ConnectionStatus::Up));
}
