//! Stopping every message to a recipient (`Notifier::stop_messages_to`, used
//! by wl-hr `record_death`, BL-HRM-09 #199): the waiting messages are
//! cancelled, a sent one stays and nothing new reaches the recipient.

mod support;

use wl_kernel::notify::{self, NotifyRecipient};
use wl_notify::MessageStatus;

use support::{BLUE_DUNES, QUIET_SANDS, REMINDER, boot};

/// A message waiting for the party is `suppressed` with the reason, a message
/// already sent stays `sent`, and another party's message is untouched.
#[renox::test]
async fn stopping_a_party_cancels_its_waiting_messages_and_keeps_the_sent_ones() {
    let t = boot().await;
    let sent = t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    assert_eq!(t.message(sent[0]).await.status, MessageStatus::Sent);
    let waiting = t.notify_party(REMINDER, BLUE_DUNES).await;
    let other = t.notify_party(REMINDER, QUIET_SANDS).await;
    assert_eq!(t.message(waiting[0]).await.status, MessageStatus::Queued);

    let state = t.app.state();
    let stopped = notify::notifier(state)
        .stop_messages_to(
            state,
            vec![NotifyRecipient::Party { id: BLUE_DUNES }],
            "employee_deceased",
        )
        .await
        .unwrap();
    assert_eq!(stopped, 1);

    let waiting = t.message(waiting[0]).await;
    assert_eq!(waiting.status, MessageStatus::Suppressed);
    assert_eq!(
        waiting.suppress_reason.as_deref(),
        Some("employee_deceased")
    );
    assert_eq!(t.message(sent[0]).await.status, MessageStatus::Sent);
    // The other party is not stopped: whatever happened to its message, it
    // was not the stop.
    assert_ne!(
        t.message(other[0]).await.suppress_reason.as_deref(),
        Some("employee_deceased")
    );
}

/// A message asked for after the stop is suppressed at once and never sent.
#[renox::test]
async fn a_message_asked_for_after_the_stop_is_suppressed_and_not_sent() {
    let t = boot().await;
    let state = t.app.state();
    notify::notifier(state)
        .stop_messages_to(
            state,
            vec![NotifyRecipient::Party { id: BLUE_DUNES }],
            "employee_deceased",
        )
        .await
        .unwrap();

    let later = t.notify_party(REMINDER, BLUE_DUNES).await;
    t.run_jobs().await;
    let later = t.message(later[0]).await;
    assert_eq!(later.status, MessageStatus::Suppressed);
    assert_eq!(later.suppress_reason.as_deref(), Some("recipient_stopped"));
    assert!(t.email.sent().is_empty() && t.sms.sent().is_empty());
}
