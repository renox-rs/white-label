//! BL-LOG-25 (#323), 03 §8 *Integration down*: while the SMS gateway is not
//! up, tracking SMS are queued; when it goes down the dispatcher gets one task
//! that lists the recipients on today's runs to call, and the clients of those
//! shipments are told by e-mail.

mod support;

use logistics::defaults::SMS_GATEWAY;
use logistics::notices::CLIENT_NOTICE;
use logistics::sms_fallback;
use support::{SOUQ, boot};
use wl_kernel::integrations::{self, Delivery, IntegrationConnection, IntegrationDef, Submitted};

/// Registers the gateway as the host does.
fn register_gateway() {
    IntegrationDef::of(SMS_GATEWAY, "notify", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .owner_role("dispatcher")
    .screens(["logistics.tracking_sms"])
    .fallback(
        logistics::defaults::SMS_FALLBACK_EN,
        Some(logistics::defaults::SMS_FALLBACK_AR),
    )
    .register();
}

/// Takes the gateway down, as the scheduled check does after its failures:
/// `go_down` waits for the connection's limit (15 minutes of failures).
async fn gateway_down(w: &support::World) {
    integrations::report_failure(w.state(), SMS_GATEWAY, "gateway timeout")
        .await
        .unwrap();
    w.app.travel(std::time::Duration::from_secs(16 * 60));
    let connection = IntegrationConnection::by_key(w.db(), SMS_GATEWAY)
        .await
        .unwrap()
        .expect("the connection");
    let down = w
        .app
        .at_travelled_time(integrations::go_down(w.state(), &connection))
        .await;
    assert!(down.unwrap());
}

/// AC5: with the gateway up a tracking SMS is sent; with it down it is queued
/// and not sent twice; the same natural key is one message.
#[renox::test]
async fn a_tracking_sms_is_queued_while_the_gateway_is_down() {
    register_gateway();
    let w = boot().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;

    let sent =
        sms_fallback::queue_tracking_sms(w.state(), &shipment, "SHP:out", "Out for delivery")
            .await
            .unwrap();
    assert_eq!(sent, Submitted::Sent);

    gateway_down(&w).await;
    let queued = sms_fallback::queue_tracking_sms(w.state(), &shipment, "SHP:next", "Delivered")
        .await
        .unwrap();
    assert!(matches!(queued, Submitted::Queued { .. }), "{queued:?}");
    let again = sms_fallback::queue_tracking_sms(w.state(), &shipment, "SHP:next", "Delivered")
        .await
        .unwrap();
    assert!(matches!(again, Submitted::Duplicate { .. }), "{again:?}");
}

/// AC5: when the gateway goes down the dispatcher has a task listing today's
/// recipients and their clients receive the e-mail; only once for the outage.
#[renox::test]
async fn the_dispatcher_gets_the_recipients_to_call_and_the_clients_an_email() {
    register_gateway();
    let w = boot().await;
    let first = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    let second = w.book_as(&w.noor, SOUQ, "ORD-2").await;
    let waiting = w.book_as(&w.noor, SOUQ, "ORD-3").await;
    let (first, _) = w.out_for_delivery(&first).await;
    let (second, _) = w.out_for_delivery(&second).await;
    let clients_before = w.notices.to_clients().len();

    gateway_down(&w).await;

    let tasks = w.tickets.opened();
    assert_eq!(tasks.len(), 1, "one task for the dispatcher");
    assert!(tasks[0].title.contains("SMS gateway is down"));
    for shipment in [&first, &second] {
        assert!(
            tasks[0].description.contains(&shipment.number),
            "{}",
            shipment.number
        );
    }
    assert!(tasks[0].description.contains("+971500000000"));
    assert!(
        !tasks[0].description.contains(&waiting.number),
        "a shipment not on today's runs is not listed"
    );
    // One e-mail to the client with both shipments.
    let told = w.notices.to_clients();
    assert_eq!(told.len(), clients_before + 1);
    let last = told.last().unwrap();
    assert_eq!(last.type_key, CLIENT_NOTICE);
    let body = last.data["body"].as_str().unwrap_or_default();
    assert!(
        body.contains(&first.number) && body.contains(&second.number),
        "{body}"
    );

    // Another change of the same connection does nothing more.
    let event = integrations::IntegrationStatusChanged {
        connection: SMS_GATEWAY.to_owned(),
        from: "up".to_owned(),
        to: "degraded".to_owned(),
        failed_since: None,
    };
    assert!(
        !sms_fallback::on_status_changed(w.state(), &event)
            .await
            .unwrap()
    );
    let other = integrations::IntegrationStatusChanged {
        connection: "email".to_owned(),
        from: "up".to_owned(),
        to: "down".to_owned(),
        failed_since: None,
    };
    assert!(
        !sms_fallback::on_status_changed(w.state(), &other)
            .await
            .unwrap()
    );
    assert_eq!(w.tickets.opened().len(), 1);
}
