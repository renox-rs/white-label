//! BL-AUT-05 (#401, `AUT.RUL.18`): vehicles that are not collected. The daily
//! run reminds the customer at 3 and 7 days (an advisor's call task when there
//! is no mobile number), gives the branch manager a task at 30 days saying
//! *legal next steps open (L-36)*, and the storage fee agreed at check-in is
//! invoiced at release when the client turned it on, never after a total loss.
//! The days are driven through `storage::process(state, day)`; the real
//! invoicing and messages are in `host/tests/auto_uncollected_real.rs`.

mod support;

use auto::release::{self, Release};
use auto::settings;
use auto::storage::{self, process};
use auto::{JobCard, JobCardStatus};
use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use support::{Garage, Messages, at, boot, day};
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::notify::NotifyChannel;

/// A job card of the Hilux in `status`, that entered the state `back` days ago,
/// with a storage fee of AED 50 a day agreed at check-in.
async fn waiting(t: &Garage, status: JobCardStatus, back: i64, code: Option<&str>) -> JobCard {
    let vehicle = t.hilux().await;
    let card = t.card_in(&vehicle, status).await;
    let when = at(day(-back), 10, 0);
    let (ready, closed) = match status {
        JobCardStatus::Ready => (Some(when), None),
        _ => (None, Some(when)),
    };
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            t.db(),
            &[
                ("storage_fee_daily", &5_000_i64),
                ("ready_at", &ready),
                ("closed_at", &closed),
                ("closed_code", &code.map(str::to_owned)),
            ],
        )
        .await
        .unwrap();
    t.reload(card.id).await
}

/// The day `n` days after `card` began to wait.
fn after(card: &JobCard, n: i64) -> NaiveDate {
    storage::start_of(card).expect("a waiting job card") + Duration::days(n)
}

/// Point 1: JC-50 is `ready` and not released: the customer is texted at 3 and
/// at 7 days, the branch manager has a task at 30 days that says the legal
/// next steps are open, and each step fires once.
#[renox::test]
async fn aut_05_the_customer_is_reminded_at_three_and_seven_days_and_the_manager_at_thirty() {
    let t = boot().await;
    let card = waiting(&t, JobCardStatus::Ready, 30, None).await;

    process(t.state(), after(&card, 2)).await.unwrap();
    assert!(t.messages.of_type("auto.collect_vehicle").is_empty());
    process(t.state(), after(&card, 3)).await.unwrap();
    process(t.state(), after(&card, 3)).await.unwrap();
    let first = t.messages.of_type("auto.collect_vehicle");
    assert_eq!(first.len(), 1, "once");
    assert!(Messages::told_party(&first[0], card.customer_party_id));
    let text = Messages::text(&first[0], NotifyChannel::Sms);
    assert!(
        text.contains("Dubai A 12345") && text.contains(&card.number()),
        "{text}"
    );
    assert!(text.contains("3 days"), "{text}");

    process(t.state(), after(&card, 6)).await.unwrap();
    assert_eq!(t.messages.of_type("auto.collect_vehicle").len(), 1);
    process(t.state(), after(&card, 7)).await.unwrap();
    let both = t.messages.of_type("auto.collect_vehicle");
    assert_eq!(both.len(), 2);
    assert!(Messages::text(&both[1], NotifyChannel::Sms).contains("7 days"));

    assert!(t.tickets.with_key("auto.uncollected:").is_empty());
    process(t.state(), after(&card, 29)).await.unwrap();
    assert!(t.tickets.with_key("auto.uncollected:").is_empty());
    process(t.state(), after(&card, 30)).await.unwrap();
    process(t.state(), after(&card, 31)).await.unwrap();
    let tasks = t.tickets.with_key("auto.uncollected:");
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0]
            .ticket
            .title
            .contains("legal next steps open (L-36)")
    );
    assert_eq!(t.messages.of_type("auto.collect_vehicle").len(), 2);
}

/// A late run fires every step whose day has passed, in order, once.
#[renox::test]
async fn aut_05_a_late_run_fires_the_missed_steps_once() {
    let t = boot().await;
    let card = waiting(&t, JobCardStatus::Declined, 12, Some("estimate_declined")).await;
    process(t.state(), after(&card, 12)).await.unwrap();
    assert_eq!(t.messages.of_type("auto.collect_vehicle").len(), 2);
    process(t.state(), after(&card, 13)).await.unwrap();
    assert_eq!(t.messages.of_type("auto.collect_vehicle").len(), 2);
    let after_run = t.reload(card.id).await;
    assert_eq!(after_run.collect_first_on, Some(after(&card, 3)));
    assert_eq!(after_run.collect_second_on, Some(after(&card, 7)));
}

/// A customer with no mobile number is not texted: the advisor gets a task to
/// call.
#[renox::test]
async fn aut_05_a_customer_without_a_mobile_gives_the_advisor_a_call_task() {
    let t = boot().await;
    let card = waiting(&t, JobCardStatus::Cancelled, 5, Some("customer_stopped")).await;
    t.people.set_no_mobile(card.customer_party_id, true);
    process(t.state(), after(&card, 3)).await.unwrap();
    assert!(t.messages.of_type("auto.collect_vehicle").is_empty());
    let calls = t.tickets.with_key("auto.collect_call:");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].ticket.title.contains(&card.number()));
}

/// Point 2: storage on at AED 50 a day and agreed at check-in, released today
/// after the first reminder fell due 6 days ago: 6 days = AED 300 + VAT, on an
/// invoice of its own.
#[renox::test]
async fn aut_05_release_invoices_the_days_from_the_first_reminder() {
    let t = boot().await;
    settings::set(t.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    let card = waiting(&t, JobCardStatus::Cancelled, 9, Some("customer_stopped")).await;
    process(t.state(), after(&card, 3)).await.unwrap();
    process(t.state(), after(&card, 7)).await.unwrap();
    let sami = t.advisor().await;
    let released = release::release(t.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert_eq!(released.storage_days, 6);
    let invoices = t.books.about(&card.subject());
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].net(), 30_000, "6 days at AED 50");
    assert_eq!(invoices[0].gross(), 31_500, "plus VAT");
    assert_eq!(released.storage_invoice_id, Some(invoices[0].id));
}

/// Point 3: storage off (the default): nothing is charged.
#[renox::test]
async fn aut_05_nothing_is_charged_while_the_storage_fee_is_off() {
    let t = boot().await;
    let card = waiting(&t, JobCardStatus::Cancelled, 9, Some("customer_stopped")).await;
    process(t.state(), after(&card, 3)).await.unwrap();
    let sami = t.advisor().await;
    let released = release::release(t.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert_eq!(released.storage_days, 0);
    assert!(t.books.about(&card.subject()).is_empty());
}

/// Nothing is charged when the customer agreed no daily rate at check-in,
/// although the switch is on.
#[renox::test]
async fn aut_05_nothing_is_charged_without_the_agreement_at_check_in() {
    let t = boot().await;
    settings::set(t.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    let card = waiting(&t, JobCardStatus::Cancelled, 9, Some("customer_stopped")).await;
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("storage_fee_daily", &0_i64)])
        .await
        .unwrap();
    process(t.state(), after(&card, 3)).await.unwrap();
    let sami = t.advisor().await;
    release::release(t.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert!(t.books.about(&card.subject()).is_empty());
}

/// Point 4: JC-52 `cancelled` for a total loss, storage on and agreed: the
/// reminders go out but no `AUT.RUL.18` storage fee is invoiced at release.
#[renox::test]
async fn aut_05_a_total_loss_gets_the_reminders_but_no_storage_fee() {
    let t = boot().await;
    settings::set(t.db(), settings::STORAGE_FEE_ENABLED, "1", None)
        .await
        .unwrap();
    let card = waiting(&t, JobCardStatus::Cancelled, 9, Some("total_loss")).await;
    process(t.state(), after(&card, 3)).await.unwrap();
    process(t.state(), after(&card, 7)).await.unwrap();
    assert_eq!(t.messages.of_type("auto.collect_vehicle").len(), 2);
    let sami = t.advisor().await;
    // After a total loss the insurer's notice, not payment, clears the release
    // for an insurance job; this job card has no claim and nothing billed.
    let released = release::release(t.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    assert!(released.released_at.is_some());
    assert!(t.books.about(&card.subject()).is_empty());
}

/// Point 5: the release was recorded before the first reminder fell due, so
/// no reminder is sent afterwards.
#[renox::test]
async fn aut_05_a_released_vehicle_is_not_reminded() {
    let t = boot().await;
    let card = waiting(&t, JobCardStatus::Declined, 1, Some("estimate_declined")).await;
    let sami = t.advisor().await;
    release::release(t.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap();
    process(t.state(), today() + Duration::days(10))
        .await
        .unwrap();
    process(t.state(), today() + Duration::days(40))
        .await
        .unwrap();
    assert!(t.messages.of_type("auto.collect_vehicle").is_empty());
    assert!(t.tickets.with_key("auto.").is_empty());
}

/// Point 6: JC-51 `declined`, its invoice unpaid and the customer on cash
/// terms: release is refused; as is without a signature is refused too.
#[renox::test]
async fn aut_05_release_is_refused_while_unpaid_and_as_is_needs_a_signature() {
    let t = boot().await;
    let card = waiting(&t, JobCardStatus::Declined, 2, Some("estimate_declined")).await;
    let draft = InvoiceDraft::new(PartyId(card.customer_party_id), card.subject()).line(
        DraftLine::new("Inspection fee", 10_000, VatCategory::Standard),
    );
    let invoice = invoicer(t.state())
        .draft(t.db(), draft)
        .await
        .unwrap()
        .expect("an invoice");
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(
            t.db(),
            &[
                ("invoice_id", &Some(invoice.0)),
                ("billed_net", &10_000_i64),
            ],
        )
        .await
        .unwrap();
    let sami = t.advisor().await;
    let unpaid = release::release(t.state(), &sami, card.id, Release::safe("Mona Hassan"))
        .await
        .unwrap_err();
    assert!(unpaid.to_string().contains("not paid"), "{unpaid}");
    let unsigned = release::release(
        t.state(),
        &sami,
        card.id,
        Release {
            released_to: "Mona Hassan".to_owned(),
            condition: "as_is".to_owned(),
            signature: None,
        },
    )
    .await
    .unwrap_err();
    assert!(unsigned.to_string().contains("signature"), "{unsigned}");
    assert!(t.reload(card.id).await.released_at.is_none());
}
