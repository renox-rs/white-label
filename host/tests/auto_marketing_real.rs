//! BL-AUT-18 (#414) point 16, the marketing and loyalty values of the 08 client
//! run on the REAL wl-notify and wl-sales of the booted demo parts trader:
//! marketing needs an opt-in, goes out 07:00-21:00 only (a message at 22:00 is
//! held to 07:00), at most 2 per party in 7 days (the third is suppressed), a
//! broadcast of 201 recipients needs a second approver and one of 200 does not;
//! a car owner's paid invoice of AED 500 earns 50 points and a fleet's earns
//! none.

mod support;

use std::sync::Arc;

use host::roles::OWNER;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use support::autofx::{World, boot, boot_with};
use support::{company, tenant_with_mobile};
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::consents::{ConsentFuture, Consents};
use wl_kernel::notify::{
    self, Category, NotificationTypeDef, NotifyChannel, NotifyRecipient, NotifyRequest,
};
use wl_kernel::{PartyId, Subject};
use wl_notify::{Message, MessageStatus};

const OFFER: &str = "auto.test_offer";

/// Marketing consent for every party on every channel.
struct Opted;

impl Consents for Opted {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        purpose: &'a str,
        _channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        let yes = purpose == "marketing";
        Box::pin(async move { Ok(yes) })
    }

    fn withdraw<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _purpose: &'a str,
        _channel: NotifyChannel,
        _source: &'a str,
    ) -> ConsentFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

fn register_offer() {
    notify::register_type(
        NotificationTypeDef::new(OFFER, "sales")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Offer"),
                "A new offer from the workshop.",
            ),
    );
}

fn at(date: NaiveDate, hour: u32, minute: u32) -> renox::db::DateTime {
    BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).expect("a time"))
}

/// One marketing message to `party` at the Asia/Dubai instant `when`, sent by
/// the job; the message as stored after.
async fn offer(w: &World, party: PartyId, when: renox::db::DateTime) -> Message {
    w.app.travel_back();
    let ahead = (when - renox::db::now()).to_std().expect("ahead");
    w.app
        .travel(std::time::Duration::from_secs(ahead.as_secs() + 1));
    let request = NotifyRequest::new(OFFER, Subject::new("kernel.legal_entity", 1))
        .to(NotifyRecipient::Party { id: party });
    let state = w.state();
    let ids = w
        .app
        .at_travelled_time(notify::notifier(state).notify(state, request))
        .await
        .expect("the notifier queues the message");
    w.app.run_all_jobs().await;
    Message::find(w.db(), ids[0].0)
        .await
        .unwrap()
        .expect("the message")
}

/// A party that never opted in gets no marketing message, whatever the hour.
#[renox::test]
async fn aut_18_real_marketing_needs_an_opt_in() {
    let w = boot().await;
    register_offer();
    let mona = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    let day = renox::db::now().date_naive() + renox::chrono::Duration::days(2);

    let sent = offer(&w, mona, at(day, 12, 0)).await;

    assert_eq!(sent.status, MessageStatus::Suppressed, "{sent:?}");
    assert_eq!(sent.suppress_reason.as_deref(), Some("no_consent"));
    assert!(sent.sent_at.is_none());
}

/// With the opt-in: two messages in 7 days go out inside the window, the third
/// is suppressed for the cap; a message at 22:00 is held to 07:00 and one at
/// 06:30 too.
#[renox::test]
async fn aut_18_real_marketing_is_capped_at_two_a_week_and_held_to_07_00_21_00() {
    let w = boot_with(|app| {
        let consents: Arc<dyn Consents> = Arc::new(Opted);
        app.provide(consents)
    })
    .await;
    register_offer();
    let mona = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    let omar = tenant_with_mobile(&w.app, "Omar Saleh", "+971507654321").await;
    let day = renox::db::now().date_naive() + renox::chrono::Duration::days(2);

    // Inside the window, at the 08 values: two go out, the third is capped.
    let first = offer(&w, mona, at(day, 10, 0)).await;
    assert_eq!(
        first.status,
        MessageStatus::Sent,
        "{:?}",
        first.suppress_reason
    );
    assert_eq!(
        offer(&w, mona, at(day, 11, 0)).await.status,
        MessageStatus::Sent
    );
    let third = offer(&w, mona, at(day, 12, 0)).await;
    assert_eq!(third.status, MessageStatus::Suppressed, "{third:?}");
    assert_eq!(third.suppress_reason.as_deref(), Some("frequency_cap"));

    // Outside the window: held until 07:00 the next morning.
    let late = offer(&w, omar, at(day, 22, 0)).await;
    assert_eq!(late.status, MessageStatus::Deferred, "{late:?}");
    assert_eq!(
        late.scheduled_for,
        Some(at(day + renox::chrono::Duration::days(1), 7, 0))
    );
    let early = offer(
        &w,
        tenant_with_mobile(&w.app, "Noor Khalid", "+971509999999").await,
        at(day, 6, 30),
    )
    .await;
    assert_eq!(early.status, MessageStatus::Deferred, "{early:?}");
    assert_eq!(early.scheduled_for, Some(at(day, 7, 0)));
    // The windows are stored for the three marketing channels.
    for channel in [
        NotifyChannel::Sms,
        NotifyChannel::Whatsapp,
        NotifyChannel::Email,
    ] {
        let window = wl_notify::SendWindow::query()
            .where_(wl_notify::SendWindow::CHANNEL.eq(channel))
            .first(w.db())
            .await
            .unwrap()
            .expect("a window");
        assert_eq!(
            (window.start_minute, window.end_minute),
            (7 * 60, 21 * 60),
            "{channel:?}"
        );
    }
}

/// A broadcast of 200 recipients is scheduled by its requester; one of 201 needs
/// a second approver: the module's own decision with the 08 limit.
#[renox::test]
async fn aut_18_real_a_broadcast_above_200_recipients_needs_a_second_approver() {
    let w = boot().await;
    let settings = wl_notify::broadcast::settings(w.state());
    assert!(!settings.needs_approval(200));
    assert!(settings.needs_approval(201));
}

/// A car owner who is a loyalty member and whose invoice of AED 500 before VAT is
/// paid earns 50 points; the fleet's paid invoice earns none, a fleet being no
/// member.
#[renox::test]
async fn aut_18_real_a_car_owner_earns_points_on_a_paid_invoice_and_a_fleet_earns_none() {
    use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoicePaid, VatCategory, invoicer};
    use wl_sales::loyalty_account::{Enrolment, enrol};
    use wl_sales::{LoyaltyAccount, LoyaltyEarning};

    let w = boot().await;
    let (_, _) = w.staff("Omar", OWNER).await;
    let owner = tenant_with_mobile(&w.app, "Mona Hassan", "+971501234567").await;
    let fleet = company(&w.app, "Al Noor Logistics").await;
    enrol(
        w.state(),
        Enrolment::new(owner, "0501234567").accepted_terms(),
    )
    .await
    .unwrap();

    let books = invoicer(w.state());
    let mut paid = Vec::new();
    for (party, reference) in [(owner, 1), (fleet, 2)] {
        let draft = InvoiceDraft::new(party, Subject::new("auto.job_card", reference))
            .line(DraftLine::new("Service", 50_000, VatCategory::Standard));
        let id = books
            .draft(w.db(), draft)
            .await
            .unwrap()
            .expect("invoicing");
        books.issue(w.state(), id, None).await.ok();
        paid.push((party, id, reference));
    }
    for (party, id, reference) in &paid {
        w.state()
            .emit(InvoicePaid {
                invoice: *id,
                party: *party,
                source: Subject::new("auto.job_card", *reference),
            })
            .await
            .unwrap();
    }

    let member = LoyaltyAccount::of_party(w.db(), owner)
        .await
        .unwrap()
        .expect("the member");
    assert_eq!(member.points_balance, 50, "AED 500 earns 50 points");
    assert!(
        LoyaltyAccount::of_party(w.db(), fleet)
            .await
            .unwrap()
            .is_none(),
        "a fleet is no member"
    );
    let earnings =
        LoyaltyEarning::of_source(w.db(), &Subject::new("invoicing.invoice", paid[1].1.0))
            .await
            .unwrap();
    assert!(earnings.is_none(), "the fleet's invoice earned nothing");
}
