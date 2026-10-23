//! BL-AUT-18 (#414), *Notifications*: the SMS that tells the customer the
//! vehicle is ready (`AUT.RUL.02`, `auto.vehicle_ready`) is sent to the
//! customer; when it gives up the contact is marked invalid by the platform,
//! the job card shows "message not delivered" to the advisor and no task is
//! created.

mod support;

use auto::JobCardStatus;
use auto::notices::{self, VEHICLE_READY};
use support::boot;
use wl_kernel::notify::{
    ChannelTried, GiveUpReaction, MessageGaveUp, MessageId, NotifyRecipient,
};
use wl_kernel::PartyId;

#[renox::test]
async fn aut_18_the_ready_sms_goes_to_the_customer_and_a_give_up_marks_the_job_card_without_a_task() {
    let t = boot().await;
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, JobCardStatus::Ready).await;

    notices::vehicle_ready(t.state(), card.id).await.unwrap();
    let sent = t.messages.of_type(VEHICLE_READY);
    assert_eq!(sent.len(), 1, "one SMS");
    assert_eq!(sent[0].subject, card.subject());

    let gave_up = MessageGaveUp {
        message: MessageId(1),
        type_key: VEHICLE_READY.to_owned(),
        subject: card.subject(),
        subject_label: None,
        recipient: NotifyRecipient::Party { id: PartyId(7) },
        gave_up_at: renox::db::now(),
        contact_point: None,
        channels_tried: Vec::<ChannelTried>::new(),
        statutory: false,
        reactions: vec![GiveUpReaction::MarkContactInvalid],
        alternative_of: None,
        contact_of: None,
    };
    notices::on_gave_up(t.state(), &gave_up).await.unwrap();

    assert!(t.reload(card.id).await.ready_message_failed_at.is_some());
    let advisor = t.advisor_user().await;
    t.app.acting_as(&advisor);
    t.app
        .get(&format!("/auto/job-cards/{}", card.id))
        .await
        .assert_ok()
        .assert_see("vehicle is ready was not delivered");
    // The collect message is not the one that failed.
    assert!(t.reload(card.id).await.collect_message_failed_at.is_none());
}
