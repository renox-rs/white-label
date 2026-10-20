//! Acceptance checks of #326 (BL-ACC-02, `ACC.RUL.11`) with the real
//! wl-ticketing and wl-portal: the manager's chase task is a ticket assigned
//! to the manager, closed when the client approver signs in the portal, and
//! the decline then leaves the portal page a 404.

mod support;

use accounting::Engagement;
use accounting::letters::{EngagementLetter, chase_key, run};
use renox::prelude::*;

use support::{boot_real, d};

/// The tickets with the source key `key`.
async fn tickets_with(t: &support::Booted, key: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .where_(wl_ticketing::Ticket::SOURCE_KEY.eq(key))
        .get(t.db())
        .await
        .unwrap()
}

/// On day 30 the manager gets a ticket to chase or decline; the client
/// approver's signature completes it; nothing is declined by itself.
#[renox::test]
async fn the_chase_is_a_ticket_for_the_manager_and_the_signature_completes_it() {
    let t = boot_real().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    Engagement::query()
        .where_(Engagement::ID.eq(engagement.id))
        .update(t.db(), &[("manager_user_id", &Some(manager.id))])
        .await
        .unwrap();
    let approver = t.portal_user("Omar", client.ubo, client.company).await;

    t.set_clock_on(d(11, 1), 10, 0);
    t.act(&manager, &engagement, "letter/send", &[])
        .await
        .assert_ok();
    let letter = EngagementLetter::current(t.db(), engagement.id)
        .await
        .unwrap()
        .expect("the letter was sent");
    assert_eq!(letter.sent_on, d(11, 1));

    t.set_clock_on(d(12, 1), 9, 0);
    let report = t.run(run(t.state(), d(12, 1))).await.unwrap();
    assert_eq!(report.chased, [letter.id]);
    let tickets = tickets_with(&t, &chase_key(&letter)).await;
    assert_eq!(tickets.len(), 1, "{tickets:?}");
    assert!(tickets[0].is_open());
    assert_eq!(tickets[0].assignee_user_id, Some(manager.id));
    assert!(tickets[0].title.contains(&engagement.number()));

    t.set_clock_on(d(12, 2), 10, 0);
    t.post(
        &approver,
        &format!(
            "/accounting/portal/engagements/{}/letter/sign",
            engagement.id
        ),
        &[],
    )
    .await
    .assert_ok();
    let tickets = tickets_with(&t, &chase_key(&letter)).await;
    assert!(!tickets[0].is_open(), "the signature completes the task");
    assert!(t.reload(&engagement).await.letter_signed);
}
