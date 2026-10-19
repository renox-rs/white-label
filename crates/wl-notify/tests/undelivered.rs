//! The "not delivered" query (#60 A7, AC9).

mod support;

use wl_kernel::Subject;
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_notify::{MessageStatus, SendError, undelivered_for};

use support::{SARA, TICKET_UPDATE, boot};

fn ticket(id: i64) -> Subject {
    Subject::new("ticketing.ticket", id)
}

#[renox::test]
async fn a_message_that_gave_up_is_listed_for_its_subject_only() {
    let t = boot().await;
    t.give_up_on_sara(TICKET_UPDATE, 42, "Lift").await;

    let found = undelivered_for(t.db(), &ticket(42)).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].type_key, TICKET_UPDATE);
    assert_eq!(found[0].message.status, MessageStatus::Abandoned);
    assert!(found[0].gave_up_at.is_some());
    assert!(
        undelivered_for(t.db(), &ticket(43))
            .await
            .unwrap()
            .is_empty()
    );
}

#[renox::test]
async fn a_message_sent_by_fallback_is_not_listed() {
    let t = boot().await;
    t.email
        .then([Err(SendError::permanent("550", "mailbox unknown"))]);
    let request =
        NotifyRequest::new(TICKET_UPDATE, ticket(44)).to(NotifyRecipient::Party { id: SARA });
    let state = t.app.state();
    t.app
        .at_travelled_time(wl_kernel::notify::notifier(state).notify(state, request))
        .await
        .unwrap();
    t.app.run_all_jobs().await;

    let messages = t.messages().await;
    assert!(
        messages
            .iter()
            .any(|m| m.status == MessageStatus::Abandoned),
        "the e-mail fell back"
    );
    assert!(
        undelivered_for(t.db(), &ticket(44))
            .await
            .unwrap()
            .is_empty()
    );
}
