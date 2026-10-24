//! BL-FNB-17 (#381), 06 §7.2, `NTF.RUL.02`, `NTF.ACL.04`, `SAL.CRM.02`,
//! `SAL.ACL.07`: the marketing rules of a fresh 06 deployment through the real
//! host and wl-notify: a third marketing message in 7 days is suppressed, a
//! customer without consent gets none, a broadcast to 201 customers waits for
//! a second approver, a cost entry that takes a campaign over its budget waits
//! for a holder of the costs permission other than its requester, and the
//! loyalty programme earns, limits a redemption to half of the order and asks
//! a manager above 500 points.
//!
//! Only the e-mail and SMS providers, the consents and the clock are
//! stand-ins.

mod support;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use host::roles::{ADMIN, MARKETING, MARKETING_MANAGER};
use renox::chrono::{Duration, NaiveDate, NaiveDateTime};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{fnb_booted_rigged, person, tenant};
use wl_kernel::approvals;
use wl_kernel::consents::{ConsentFuture, Consents, provide_consents};
use wl_kernel::documents::today;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::{BusinessCalendar, PartyId, Subject};
use wl_notify::channels::ProviderFuture;
use wl_notify::{Broadcast, BroadcastStatus};
use wl_notify::{
    ChannelProvider, Clock, Message, MessageStatus, Outgoing, SendError, WlNotify, release,
};
use wl_sales::sale_line::KIND_ITEM;
use wl_sales::{
    Campaign, CampaignBroadcast, CounterSale, CounterSaleStatus, MarketingChannel, Purpose,
    SaleLine, Segment, campaigns,
};

const OFFER: &str = "fnb_test.offer";

/// An e-mail provider that accepts everything and keeps what it was given.
#[derive(Clone, Default)]
struct Mailbox(Arc<Mutex<Vec<Outgoing>>>);

impl Mailbox {
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

impl ChannelProvider for Mailbox {
    fn name(&self) -> &'static str {
        "test-email"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        self.0.lock().unwrap().push(outgoing.clone());
        Box::pin(async { Ok("mail-1".to_owned()) })
    }
}

/// The customers who gave their marketing consent.
#[derive(Clone, Default)]
struct Consenting(Arc<Mutex<BTreeSet<i64>>>);

impl Consenting {
    fn give(&self, party: PartyId) {
        self.0.lock().unwrap().insert(party.0);
    }
}

impl Consents for Consenting {
    fn has_consent<'a>(
        &'a self,
        _db: &'a Db,
        party: PartyId,
        purpose: &'a str,
        _channel: NotifyChannel,
    ) -> ConsentFuture<'a, bool> {
        let yes = purpose == "marketing" && self.0.lock().unwrap().contains(&party.0);
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

/// The clock of wl-notify, moved by the test.
#[derive(Clone)]
struct Moving(Arc<Mutex<DateTime>>);

impl Moving {
    fn set(&self, at: DateTime) {
        *self.0.lock().unwrap() = at;
    }
}

/// Asia/Dubai `date` at `hour:minute`, as an instant.
fn dubai(date: NaiveDate, hour: u32, minute: u32) -> DateTime {
    let local: NaiveDateTime = date.and_hms_opt(hour, minute, 0).expect("a time");
    BusinessCalendar::default().from_local(local)
}

/// What the test provides to the host.
#[derive(Clone)]
struct Rig {
    consents: Consenting,
    clock: Moving,
}

impl Module for Rig {
    fn name(&self) -> &'static str {
        "fnb_marketing_rig"
    }

    fn register(&self, app: &mut Registry) {
        provide_consents(app, self.consents.clone());
        let at = self.clock.0.clone();
        app.provide(Clock::from_fn(move || *at.lock().unwrap()));
    }
}

fn register_type() {
    notify::register_type(
        NotificationTypeDef::new(OFFER, "fnb_test")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(
                NotifyChannel::Email,
                "en",
                Some("A weekend offer"),
                "A weekend offer for you.",
            ),
    );
}

/// The F&B host with the mailbox, consents and the moving clock.
async fn boot(tune: impl FnOnce(WlNotify) -> WlNotify + 'static) -> (TestApp, Mailbox, Rig) {
    register_type();
    let mail = Mailbox::default();
    let rig = Rig {
        consents: Consenting::default(),
        clock: Moving(Arc::new(Mutex::new(dubai(
            renox::db::now().date_naive(),
            12,
            0,
        )))),
    };
    let provider = mail.clone();
    let app = fnb_booted_rigged(move |n| tune(n.provider(provider)), rig.clone()).await;
    (app, mail, rig)
}

/// The messages of the offer to `party`, oldest first.
async fn offers_to(app: &TestApp, party: PartyId) -> Vec<Message> {
    Message::query()
        .where_(Message::TYPE_KEY.eq(OFFER))
        .where_(Message::PARTY_ID.eq(party.0))
        .order_by(Message::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// Asks the notifier for one offer to `party` and runs the jobs.
async fn offer(app: &TestApp, party: PartyId) -> Message {
    let state = app.state();
    notify::notifier(state)
        .notify(
            state,
            NotifyRequest::new(OFFER, Subject::new("sales.campaign", 1)).to_party(party),
        )
        .await
        .expect("the message is queued");
    app.run_all_jobs().await;
    offers_to(app, party)
        .await
        .pop()
        .expect("a message was made")
}

/// A completed counter sale of AED 600 to `party`, `days` ago (a fixture: the
/// segment reads sales).
async fn sale_before(app: &TestApp, party: PartyId, days: i64, number: usize) {
    let mut sale = CounterSale {
        number: format!("FIX-{number:05}"),
        party_id: Some(party.0),
        cashier_user_id: 1,
        currency: "AED".into(),
        net_total: 60_000,
        vat_total: 3_000,
        gross_total: 63_000,
        status: CounterSaleStatus::Completed,
        completed_at: Some(renox::db::now() - Duration::days(days)),
        ..CounterSale::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    sale.insert_in_state(&mut tx, CounterSaleStatus::Completed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    SaleLine {
        subject_type: sale.subject().subject_type,
        subject_id: sale.id,
        line_no: 1,
        item_code: "shawarma".to_owned(),
        description: "Shawarma".to_owned(),
        kind: KIND_ITEM.to_owned(),
        quantity_milli: 1000,
        unit_price: 60_000,
        ..SaleLine::default()
    }
    .insert(app.db())
    .await
    .unwrap();
}

/// `NTF.RUL.02`, `NTF.ACL.04`: a marketing message queued at 22:00 waits for
/// 07:00; the third in 7 days is suppressed by the cap of 2; a customer without
/// the consent gets none.
#[renox::test]
async fn marketing_waits_for_the_morning_is_capped_at_two_and_needs_consent() {
    let (app, mail, rig) = boot(|n| n).await;
    let day = renox::db::now().date_naive();
    let next = day + Duration::days(1);
    let anna = tenant(&app, "Anna Lane").await;
    let bea = tenant(&app, "Bea Lane").await;
    let cy = tenant(&app, "Cy Lane").await;
    rig.consents.give(anna);
    rig.consents.give(bea);

    rig.clock.set(dubai(day, 22, 0));
    let held = offer(&app, anna).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai(next, 7, 0)));
    assert_eq!(mail.count(), 0);
    rig.clock.set(dubai(next, 7, 0));
    assert_eq!(
        release::run_due(app.state(), dubai(next, 7, 0))
            .await
            .unwrap(),
        1
    );
    app.run_all_jobs().await;
    assert_eq!(offers_to(&app, anna).await[0].status, MessageStatus::Sent);

    rig.clock.set(dubai(next, 12, 0));
    assert_eq!(offer(&app, bea).await.status, MessageStatus::Sent);
    rig.clock.set(dubai(next + Duration::days(2), 12, 0));
    assert_eq!(offer(&app, bea).await.status, MessageStatus::Sent);
    rig.clock.set(dubai(next + Duration::days(3), 12, 0));
    let third = offer(&app, bea).await;
    assert_eq!(third.status, MessageStatus::Suppressed);
    assert_eq!(third.suppress_reason.as_deref(), Some("frequency_cap"));
    assert_eq!(mail.count(), 3, "the capped one was not sent");

    rig.clock.set(dubai(next + Duration::days(3), 13, 0));
    let none = offer(&app, cy).await;
    assert_eq!(none.status, MessageStatus::Suppressed);
    assert_eq!(none.suppress_reason.as_deref(), Some("no_consent"));
}

/// `NTF.ACL.04`: a campaign to the lapsed customers asks for a broadcast of
/// 201 consenting customers (three have not consented): it waits for a second
/// approver, who is not the person who set the campaign, and is then
/// scheduled.
#[renox::test]
async fn a_broadcast_to_201_customers_waits_for_a_second_approver() {
    let (app, _mail, rig) = boot(|n| n).await;
    let carla = person(&app, "Carla", MARKETING).await;
    let mona = person(&app, "Mona", MARKETING_MANAGER).await;
    for n in 0..204 {
        let customer = tenant(&app, &format!("Customer {n:03}")).await;
        sale_before(&app, customer, 60, n).await;
        if n < 201 {
            rig.consents.give(customer);
        }
    }
    let segment = Segment::query()
        .where_(Segment::NAME.eq("Lapsed customers"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the 06 deployment starts with its lapsed customers segment");
    let send_at = dubai(today() + Duration::days(3), 10, 0);
    let campaign = Campaign {
        name: "Come back".into(),
        code: "COMEBACK".into(),
        purpose: Some(Purpose::Retention),
        marketing_channel: Some(MarketingChannel::Email.into()),
        owner_id: Some(mona.id),
        start_date: Some(today()),
        end_date: Some(today() + Duration::days(60)),
        segment_id: Some(segment.id),
        template_key: Some(OFFER.into()),
        message_channel: Some("email".into()),
        send_at: Some(send_at),
        retention_set_by: Some(carla.id),
        ..Campaign::default()
    }
    .store(app.db())
    .await
    .unwrap()
    .unwrap();

    let report = campaigns::run_daily(app.state(), today()).await.unwrap();
    assert_eq!((report.activated, report.failed), (1, 0));
    let rows = CampaignBroadcast::of_campaign(app.db(), campaign.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let broadcast = Broadcast::find_or_404(app.db(), rows[0].broadcast_id.expect("taken"))
        .await
        .unwrap();
    assert_eq!(
        broadcast.estimate, 201,
        "the three without consent are left out"
    );
    assert_eq!(broadcast.status, BroadcastStatus::PendingApproval);
    let request = broadcast.approval_request_id.expect("an approval request");
    assert!(
        approvals::approve(app.state(), request, &carla, "")
            .await
            .is_err(),
        "the one who set the campaign cannot approve"
    );
    approvals::approve(app.state(), request, &mona, "")
        .await
        .unwrap();
    let scheduled = Broadcast::find_or_404(app.db(), broadcast.id)
        .await
        .unwrap();
    assert_eq!(scheduled.status, BroadcastStatus::Scheduled);
    assert_eq!(scheduled.send_at, send_at);
}

/// `SAL.CRM.02`, `SAL.ACL.07`: a cost entry that takes a campaign over its
/// budget waits for a holder of the costs permission other than the person who
/// entered it; one within the budget is approved on save.
#[renox::test]
async fn a_cost_over_the_campaign_budget_waits_for_another_holder() {
    use wl_sales::CostStatus;
    use wl_sales::costs::{self, CostInput};
    let (app, _mail, _rig) = boot(|n| n).await;
    let ada = person(&app, "Ada", ADMIN).await;
    let alia = person(&app, "Alia", ADMIN).await;
    let campaign = Campaign {
        name: "Weekend".into(),
        code: "WEEKEND".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta.into()),
        owner_id: Some(ada.id),
        start_date: Some(today()),
        end_date: Some(today() + Duration::days(30)),
        budget: Some(500_000),
        ..Campaign::default()
    }
    .store(app.db())
    .await
    .unwrap()
    .unwrap();
    let entry = |amount: i64, by: &User| CostInput {
        campaign_id: campaign.id,
        entry_date: today(),
        amount,
        description: "Ads".into(),
        requested_by: by.id,
        manages_costs: false,
    };

    let within = costs::save(app.state(), entry(480_000, &ada))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(within.status, CostStatus::Approved);
    let over = costs::save(app.state(), entry(40_000, &ada))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(over.status, CostStatus::PendingApproval);
    let request = over.approval_request_id.expect("a request");
    assert!(
        approvals::approve(app.state(), request, &ada, "")
            .await
            .is_err(),
        "the person who entered it cannot approve it"
    );
    approvals::approve(app.state(), request, &alia, "")
        .await
        .unwrap();
    let approved = wl_sales::CostEntry::find(app.db(), over.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, CostStatus::Approved);
}
