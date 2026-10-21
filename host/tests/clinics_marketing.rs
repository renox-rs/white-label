//! BL-CLN-15 (#363): the marketing rules of 05 §7.2 through the real host of
//! the demo clinic with the real wl-notify: a marketing message queued at
//! 22:00 waits for 07:00, a third in 7 days is suppressed, a patient without
//! consent is left out, and a broadcast to 201 patients needs a second
//! approver; the advertising permit that ends a campaign on its last valid day
//! cancels the broadcast scheduled after it (`NTF.ACL.04`, `CLN.ACL.13`).
//!
//! Only the e-mail provider, the consents and the clock are stand-ins: the
//! host has no consent module yet (`PTY.CNS.01`), so a `Consenting` set stands
//! for the patients who agreed.

mod support;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use renox::chrono::{Duration, NaiveDate, NaiveDateTime};
use renox::db::DateTime;
use renox::prelude::*;
use renox::testing::TestApp;
use host::roles::{MARKETING, MARKETING_MANAGER, OWNER};
use renox::state_machine::StateMachine;
use support::{clinics_booted_with, person, tenant};
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_notify::broadcast_run::run_due;
use wl_notify::{Broadcast, BroadcastStatus};
use wl_sales::sale_line::KIND_ITEM;
use wl_sales::{
    BroadcastState, Campaign, CampaignBroadcast, CampaignStatus, CounterSale, CounterSaleStatus,
    MarketingChannel, Purpose, SaleLine, Segment, campaigns,
};
use wl_kernel::consents::{ConsentFuture, Consents, provide_consents};
use wl_kernel::notify::{
    self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest,
};
use wl_kernel::{BusinessCalendar, PartyId, Subject};
use wl_notify::channels::ProviderFuture;
use wl_notify::{
    ChannelProvider, Clock, Message, MessageStatus, Outgoing, SendError, WlNotify, release,
};

const OFFER: &str = "clinics_test.offer";

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

/// The patients who gave their marketing consent.
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
        "clinics_marketing_rig"
    }

    fn register(&self, app: &mut Registry) {
        provide_consents(app, self.consents.clone());
        let at = self.clock.0.clone();
        app.provide(Clock::from_fn(move || *at.lock().unwrap()));
    }
}

fn register_type() {
    notify::register_type(
        NotificationTypeDef::new(OFFER, "clinics_test")
            .group("marketing")
            .category(Category::Marketing)
            .channels(&[NotifyChannel::Email])
            .template(
                NotifyChannel::Email,
                "en",
                Some("A check-up offer"),
                "A check-up offer for you.",
            ),
    );
}

/// The clinics host with the mailbox, consents and the moving clock.
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
    let app = clinics_booted_with(
        move |n| tune(n.provider(provider)),
        rig.clone(),
    )
    .await;
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

/// Done when (sales, loyalty and retention), the notification points: a
/// marketing message queued at 22:00 is deferred to 07:00 and sent when it is
/// released; a third in 7 days is suppressed by the frequency cap; a patient
/// without marketing consent gets none.
#[renox::test]
async fn marketing_waits_for_the_morning_is_capped_and_needs_consent() {
    let (app, mail, rig) = boot(|n| n).await;
    let day = renox::db::now().date_naive();
    let next = day + Duration::days(1);
    let anna = tenant(&app, "Anna Lane").await;
    let bea = tenant(&app, "Bea Lane").await;
    let cy = tenant(&app, "Cy Lane").await;
    rig.consents.give(anna);
    rig.consents.give(bea);

    // 22:00: held until 07:00 the next day; the provider is not called.
    rig.clock.set(dubai(day, 22, 0));
    let held = offer(&app, anna).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert_eq!(held.scheduled_for, Some(dubai(next, 7, 0)));
    assert_eq!(mail.count(), 0);
    rig.clock.set(dubai(next, 7, 0));
    assert_eq!(release::run_due(app.state(), dubai(next, 7, 0)).await.unwrap(), 1);
    app.run_all_jobs().await;
    assert_eq!(offers_to(&app, anna).await[0].status, MessageStatus::Sent);
    assert_eq!(mail.count(), 1);

    // Two in a week are sent, the third is suppressed.
    rig.clock.set(dubai(next, 12, 0));
    assert_eq!(offer(&app, bea).await.status, MessageStatus::Sent);
    rig.clock.set(dubai(next + Duration::days(2), 12, 0));
    assert_eq!(offer(&app, bea).await.status, MessageStatus::Sent);
    rig.clock.set(dubai(next + Duration::days(3), 12, 0));
    let third = offer(&app, bea).await;
    assert_eq!(third.status, MessageStatus::Suppressed);
    assert_eq!(third.suppress_reason.as_deref(), Some("frequency_cap"));
    assert_eq!(mail.count(), 3, "the capped one was not sent");

    // No consent, no message.
    rig.clock.set(dubai(next + Duration::days(3), 13, 0));
    let none = offer(&app, cy).await;
    assert_eq!(none.status, MessageStatus::Suppressed);
    assert_eq!(none.suppress_reason.as_deref(), Some("no_consent"));
    assert_eq!(mail.count(), 3);
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
        item_code: "visit".to_owned(),
        description: "Visit".to_owned(),
        kind: KIND_ITEM.to_owned(),
        quantity_milli: 1000,
        unit_price: 60_000,
        ..SaleLine::default()
    }
    .insert(app.db())
    .await
    .unwrap();
}

/// Done when (retention campaign): a retention broadcast to 201 patients
/// waits for a second approver, who is not the coordinator who set it, and
/// leaves out the patients without consent; a campaign whose permit is valid
/// through day D runs through D, is ended by the first daily run after it, and
/// its broadcast scheduled for D + 2 is cancelled, through the real wl-notify.
///
/// Decision: 05 says the campaign is ended "on the day the permit's validity
/// ends"; the permit is valid through that whole day, so the system ends the
/// campaign in the first daily run after it (D + 1 at 02:00): the campaign
/// still runs on D and never sends after it.
#[renox::test]
async fn a_retention_broadcast_to_201_needs_a_second_approver_and_the_permit_end_cancels_it() {
    let (app, _mail, rig) = boot(|n| n).await;
    let carla = person(&app, "Carla", MARKETING).await;
    let mona = person(&app, "Mona", MARKETING_MANAGER).await;
    let olga = person(&app, "Olga", OWNER).await;
    for n in 0..204 {
        let patient = tenant(&app, &format!("Patient {n:03}")).await;
        sale_before(&app, patient, 200, n).await;
        // 201 agreed; three did not.
        if n < 201 {
            rig.consents.give(patient);
        }
    }
    let last_day = today() + Duration::days(10);
    let send_at = dubai(last_day + Duration::days(2), 10, 0);
    let segment = Segment::query()
        .where_(Segment::NAME.eq(clinics::seed::LAPSED_SEGMENT_NAME))
        .first(app.db())
        .await
        .unwrap()
        .expect("the clinic starts with its lapsed-patients segment");
    let campaign = Campaign {
        name: "Come back".into(),
        code: "COMEBACK".into(),
        purpose: Some(Purpose::Retention),
        marketing_channel: Some(MarketingChannel::Email),
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

    // The medical director records the permit, valid through D.
    app.acting_as(&olga);
    let id = campaign.id.to_string();
    let valid_to = last_day.to_string();
    app.htmx()
        .post(
            "/clinics/campaign-permits",
            &[
                ("campaign", id.as_str()),
                ("promotes", "yes"),
                ("permit_number", "AD-2026-77"),
                ("valid_to", valid_to.as_str()),
            ],
        )
        .await
        .assert_ok();

    // Activated, the campaign asks for its broadcast: 201 consenting
    // patients is above the limit of 200, so it waits for a second person.
    let report = campaigns::run_daily(app.state(), today()).await.unwrap();
    assert_eq!((report.activated, report.failed), (1, 0));
    let rows = CampaignBroadcast::of_campaign(app.db(), campaign.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let broadcast = Broadcast::find_or_404(app.db(), rows[0].broadcast_id.expect("taken"))
        .await
        .unwrap();
    assert_eq!(broadcast.estimate, 201, "the three without consent are left out");
    assert_eq!(broadcast.status, BroadcastStatus::PendingApproval);
    assert_eq!(broadcast.requested_by, carla.id);
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
    let scheduled = Broadcast::find_or_404(app.db(), broadcast.id).await.unwrap();
    assert_eq!(scheduled.status, BroadcastStatus::Scheduled);
    assert_eq!(scheduled.send_at, send_at);

    // On the last valid day it still runs; the day after, it ends and the
    // broadcast is cancelled.
    // (The demo data has campaigns of its own that end meanwhile: read ours.)
    campaigns::run_daily(app.state(), last_day).await.unwrap();
    let running = Campaign::find(app.db(), campaign.id).await.unwrap().unwrap();
    assert_eq!(running.status, CampaignStatus::Active);
    campaigns::run_daily(app.state(), last_day + Duration::days(1))
        .await
        .unwrap();
    let ended = Campaign::find(app.db(), campaign.id).await.unwrap().unwrap();
    assert_eq!(ended.status, CampaignStatus::Ended);
    let cancelled = Broadcast::find_or_404(app.db(), broadcast.id).await.unwrap();
    assert_eq!(cancelled.status, BroadcastStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("campaign_ended"));
    let rows = CampaignBroadcast::of_campaign(app.db(), campaign.id)
        .await
        .unwrap();
    assert_eq!(rows[0].status, BroadcastState::Cancelled);

    // The send time comes: nothing goes out.
    rig.clock.set(send_at + Duration::minutes(1));
    run_due(app.state(), send_at + Duration::minutes(1))
        .await
        .unwrap();
    app.run_all_jobs().await;
    // (The mailbox also holds the notices to staff: count the offers.)
    let offers = Message::query()
        .where_(Message::TYPE_KEY.eq(OFFER))
        .count(app.db())
        .await
        .unwrap();
    assert_eq!(offers, 0, "no offer was queued");
}
