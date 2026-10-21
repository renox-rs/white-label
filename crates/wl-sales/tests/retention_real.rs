//! Retention campaigns through the real wl-notify (BL-SAL-25, #144): the
//! broadcast is estimated with consent, scheduled without approval within the
//! limit, run, capped and reported; the approval above the limit cannot be
//! given by the user who set the campaign; a rejection shows *not sent*;
//! ending a campaign cancels a scheduled broadcast. Only the SMS provider, the
//! party directory and the consents are stand-ins.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::{PartyId, Subject};
use wl_notify::broadcast_run::run_due;
use wl_notify::channels::ProviderFuture;
use wl_notify::permissions as notify_perm;
use wl_notify::{
    Broadcast, BroadcastStatus, ChannelProvider, Clock, Outgoing, SendError, WlNotify,
};
use wl_sales::permissions as perm;
use wl_sales::retention;
use wl_sales::{
    BroadcastState, Campaign, CampaignBroadcast, CampaignStatus, MarketingChannel, MatchMode,
    Purpose, Rule, SalesConfig, Segment, WlSales, campaigns,
};

use support::retention::{Parties, sale_at};
use support::{Booted, boot_pair};

/// What the SMS provider was handed.
#[derive(Clone, Default)]
struct Phone(Arc<Mutex<Vec<Outgoing>>>);

impl Phone {
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

struct PhoneProvider(Phone);

impl ChannelProvider for PhoneProvider {
    fn name(&self) -> &'static str {
        "test-sms"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Sms
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        _state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        (self.0).0.lock().unwrap().push(outgoing.clone());
        Box::pin(async { Ok("sms-1".to_owned()) })
    }
}

/// The party directory and the consents, and a clock fixed at noon in Dubai
/// (inside every send window).
#[derive(Clone)]
struct Fakes {
    parties: Parties,
    noon: DateTime,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "real_fakes"
    }

    fn register(&self, app: &mut Registry) {
        self.parties.register(app);
        app.provide(Clock::fixed(self.noon));
    }
}

const OFFER: &str = "promo.offer";
const OFFER_2: &str = "promo.offer2";

fn register_types() {
    for key in [OFFER, OFFER_2] {
        notify::register_type(
            NotificationTypeDef::new(key, "sales")
                .group("marketing")
                .category(Category::Marketing)
                .channels(&[NotifyChannel::Sms])
                .template(NotifyChannel::Sms, "en", None, "A new offer for you."),
        );
    }
}

struct World {
    t: Booted,
    fakes: Fakes,
    phone: Phone,
    a: User,
    r: Campaign,
}

/// A client with real wl-sales and wl-notify; `members` lapsed customers
/// with ids 1.. and the campaign R set up by A. The notify settings are
/// `tune`d.
async fn world(members: i64, tune: impl FnOnce(WlNotify) -> WlNotify) -> World {
    register_types();
    let phone = Phone::default();
    let notify = tune(
        WlNotify::new()
            .provider(PhoneProvider(phone.clone()))
            .sms_sender_id("AD-SHOP"),
    );
    let noon = (today().and_hms_opt(12, 0, 0).unwrap() - Duration::hours(4)).and_utc();
    let fakes = Fakes {
        parties: Parties::default(),
        noon,
    };
    let t = boot_pair(WlSales::new(), notify, fakes.clone()).await;
    approvals::install(t.db()).await.unwrap();
    let at = now();
    for party in 1..=members {
        sale_at(&t, party, at - Duration::days(90), 60_000, "tee").await;
    }
    let a = t
        .user(
            "Aisha",
            &[
                perm::CAMPAIGNS_VIEW,
                perm::CAMPAIGNS_MANAGE,
                notify_perm::MARKETING_SEND,
            ],
        )
        .await;
    let owner = t.user("Owner", &[]).await;
    let s = Segment::of(
        "Lapsed",
        MatchMode::All,
        &[
            Rule::NoSaleForDays { days: 60 },
            Rule::SpendOver {
                amount: 50_000,
                days: 365,
            },
        ],
    )
    .store(t.db(), &SalesConfig::default())
    .await
    .unwrap()
    .unwrap();
    let send = DateTime::from_timestamp((at + Duration::hours(2)).timestamp() / 60 * 60, 0).unwrap();
    let r = Campaign {
        name: "Win back".into(),
        code: "WINBACK".into(),
        purpose: Some(Purpose::Retention),
        marketing_channel: Some(MarketingChannel::Sms),
        owner_id: Some(owner.id),
        start_date: Some(today()),
        end_date: Some(today() + Duration::days(30)),
        segment_id: Some(s.id),
        template_key: Some(OFFER.into()),
        message_channel: Some("sms".into()),
        send_at: Some(send),
        retention_set_by: Some(a.id),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap();
    World {
        t,
        fakes,
        phone,
        a,
        r,
    }
}

impl World {
    async fn start(&self) {
        let report = campaigns::run_daily(self.t.state(), today()).await.unwrap();
        assert_eq!((report.activated, report.failed), (1, 0));
    }

    async fn rows(&self) -> Vec<CampaignBroadcast> {
        CampaignBroadcast::of_campaign(self.t.db(), self.r.id)
            .await
            .unwrap()
    }

    async fn broadcast(&self, id: i64) -> Broadcast {
        Broadcast::find_or_404(self.t.db(), id).await.unwrap()
    }

    async fn reload(&self) -> Campaign {
        Campaign::find(self.t.db(), self.r.id).await.unwrap().unwrap()
    }

    /// The send time passes: wl-notify queues the messages and the jobs
    /// send them.
    async fn send(&self, send_at: DateTime) {
        run_due(self.t.state(), send_at + Duration::minutes(1))
            .await
            .unwrap();
        self.t.app.run_all_jobs().await;
    }
}

#[renox::test]
async fn the_broadcast_is_estimated_by_consent_scheduled_without_approval_and_reported() {
    let w = world(300, |n| n).await;
    // 180 of the 300 gave SMS marketing consent.
    for party in 1..=180 {
        w.fakes.parties.consent.give(party);
    }
    // 20 of them already got two marketing messages this week: the frequency
    // cap holds the third.
    for party in 1..=20 {
        for _ in 0..2 {
            notify::notifier(w.t.state())
                .notify(
                    w.t.state(),
                    NotifyRequest::new(OFFER_2, Subject::new("sales.campaign", 999))
                        .to_party(PartyId(party)),
                )
                .await
                .unwrap();
        }
    }
    w.t.app.run_all_jobs().await;
    assert_eq!(w.phone.count(), 40);

    w.start().await;
    let rows = w.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, BroadcastState::Requested);
    let b = w.broadcast(rows[0].broadcast_id.expect("wl-notify took it")).await;
    // The estimate counts only members who pass the checks: 180, within the
    // limit of 200, so no approval.
    assert_eq!(b.estimate, 180);
    assert_eq!(b.status, BroadcastStatus::Scheduled);
    assert_eq!(b.approval_request_id, None);
    assert_eq!(b.requested_by, w.a.id);

    w.send(b.send_at).await;
    let b = w.broadcast(b.id).await;
    assert_eq!(b.status, BroadcastStatus::Completed);
    assert_eq!(w.rows().await[0].status, BroadcastState::Completed);

    // Customers 1 and 21 bought after the send; so did 250, who never agreed.
    // Customer 1's message was held by the cap, so it does not count.
    let day_after = b.send_at + Duration::days(1);
    sale_at(&w.t, 1, day_after, 10_000, "tee").await;
    sale_at(&w.t, 21, day_after, 15_000, "tee").await;
    sale_at(&w.t, 250, day_after, 8_000, "tee").await;

    let report = retention::report(w.t.state(), &w.reload().await).await.unwrap();
    assert_eq!(report.status.as_deref(), Some("completed"));
    assert_eq!(report.recipients, Some(180));
    assert_eq!(report.excluded.get("no_consent"), Some(&120));
    assert_eq!(report.excluded.get("frequency_cap"), Some(&20));
    assert_eq!((report.returned, report.returned_value), (1, 15_000));
    // 40 earlier messages and 160 sent now; 20 were held.
    assert_eq!(w.phone.count(), 40 + 160);
}

#[renox::test]
async fn above_the_limit_the_user_who_set_the_campaign_cannot_approve_and_a_rejection_shows_not_sent()
 {
    let w = world(60, |n| n.broadcast_approval_above(50)).await;
    for party in 1..=60 {
        w.fakes.parties.consent.give(party);
    }
    let b_user = w
        .t
        .user(
            "Bilal",
            &[
                perm::CAMPAIGNS_VIEW,
                perm::CAMPAIGNS_MANAGE,
                notify_perm::MARKETING_SEND,
            ],
        )
        .await;
    let c_user = w
        .t
        .user("Chen", &[perm::CAMPAIGNS_VIEW, notify_perm::MARKETING_SEND])
        .await;

    w.start().await;
    let first = w.rows().await[0].broadcast_id.unwrap();
    let b = w.broadcast(first).await;
    assert_eq!(b.status, BroadcastStatus::PendingApproval);
    assert_eq!(b.estimate, 60);
    assert_eq!(b.requested_by, w.a.id);
    let request = b.approval_request_id.expect("an approval request");
    // A set the campaign, so A is the requester and cannot approve it.
    assert!(
        approvals::approve(w.t.state(), request, &w.a, "")
            .await
            .is_err()
    );

    // B rejects: the campaign stays active and shows *not sent* with why.
    approvals::reject(w.t.state(), request, &b_user, "wrong audience")
        .await
        .unwrap();
    assert_eq!(w.broadcast(first).await.status, BroadcastStatus::Cancelled);
    let rows = w.rows().await;
    assert_eq!(rows[0].status, BroadcastState::Cancelled);
    assert_eq!(rows[0].reason.as_deref(), Some("wrong audience"));
    assert_eq!(w.reload().await.status, CampaignStatus::Active);
    w.t.app.acting_as(&b_user);
    w.t.app
        .get(&format!("/campaigns/{}", w.r.id))
        .await
        .assert_ok()
        .assert_see("Not sent")
        .assert_see("wrong audience");

    // A changes the template; the campaign asks again and C approves.
    let mut fixed = w.reload().await;
    fixed.template_key = Some(OFFER_2.into());
    fixed.store(w.t.db()).await.unwrap().unwrap();
    w.t.app
        .post(&format!("/campaigns/{}/broadcast", w.r.id), &[("x", "")])
        .await
        .assert_status(303);
    let rows = w.rows().await;
    assert_eq!(rows.len(), 2);
    let second = w.broadcast(rows[1].broadcast_id.unwrap()).await;
    assert_eq!(second.status, BroadcastStatus::PendingApproval);
    assert_eq!(second.type_key, OFFER_2);
    assert_eq!(second.requested_by, w.a.id);
    approvals::approve(
        w.t.state(),
        second.approval_request_id.unwrap(),
        &c_user,
        "",
    )
    .await
    .unwrap();
    assert_eq!(w.broadcast(second.id).await.status, BroadcastStatus::Scheduled);
    w.send(second.send_at).await;
    assert_eq!(w.broadcast(second.id).await.status, BroadcastStatus::Completed);
    assert_eq!(w.rows().await[1].status, BroadcastState::Completed);
    assert_eq!(w.phone.count(), 60);

    // A third request is refused: one completed broadcast is all a campaign
    // has.
    let third = retention::request_broadcast(w.t.state(), w.r.id)
        .await
        .unwrap();
    assert_eq!(third.unwrap_err(), retention::RetentionRefused::AlreadySent);
}

#[renox::test]
async fn ending_the_campaign_before_the_send_time_cancels_the_scheduled_broadcast() {
    let w = world(5, |n| n).await;
    for party in 1..=5 {
        w.fakes.parties.consent.give(party);
    }
    w.start().await;
    let id = w.rows().await[0].broadcast_id.unwrap();
    let b = w.broadcast(id).await;
    assert_eq!(b.status, BroadcastStatus::Scheduled);

    let boss = w
        .t
        .user("Bilal", &[perm::CAMPAIGNS_VIEW, perm::CAMPAIGNS_MANAGE])
        .await;
    w.t.app.acting_as(&boss);
    w.t.app
        .post(&format!("/campaigns/{}/transitions/end", w.r.id), &[("x", "")])
        .await
        .assert_status(303);
    let cancelled = w.broadcast(id).await;
    assert_eq!(cancelled.status, BroadcastStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("campaign_ended"));
    let rows = w.rows().await;
    assert_eq!(rows[0].status, BroadcastState::Cancelled);
    assert_eq!(rows[0].reason.as_deref(), Some("campaign_ended"));

    // The send time passes: nothing goes out.
    w.send(b.send_at).await;
    assert_eq!(w.phone.count(), 0);
}
