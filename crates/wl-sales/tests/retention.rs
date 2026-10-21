//! Retention campaigns (BL-SAL-25, #144), with the other modules answered by
//! hand: activating asks for one broadcast naming the user who set the
//! fields, a refused or rejected one shows *not sent* and can be asked again
//! once, `reopen` sends nothing, ending cancels a waiting broadcast, and the
//! report counts who came back. The real wl-notify is in `retention_real.rs`.

mod support;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use renox::chrono::{Duration, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::broadcast::{
    BroadcastCancelReason, BroadcastCancelled, BroadcastCompleted, BroadcastId, BroadcastReport,
    BroadcastRequesters, RequesterCheck,
};
use wl_kernel::documents::today;
use wl_kernel::notify::NotifyChannel;
use wl_sales::permissions as perm;
use wl_sales::retention::{self, CampaignRequesters};
use wl_sales::{
    BroadcastState, Campaign, CampaignBroadcast, CampaignGuard, CampaignStatus, CostEntry,
    CostStatus, MarketingChannel, MatchMode, Purpose, Rule, SalesConfig, Segment, WlSales,
    campaigns,
};

use support::retention::{Stage, campaign_subject, sale_at};
use support::{Booted, boot_configured};

/// A moment without seconds, as the form keeps it.
fn whole_minute(at: DateTime) -> DateTime {
    DateTime::from_timestamp(at.timestamp() / 60 * 60, 0).unwrap()
}

/// A moment as the form shows it, Asia/Dubai wall time.
fn form_time(at: DateTime) -> String {
    (at + Duration::hours(4))
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

async fn lapsed(t: &Booted) -> Segment {
    Segment::of(
        "Lapsed",
        MatchMode::All,
        &[Rule::NoSaleForDays { days: 60 }],
    )
    .store(t.db(), &SalesConfig::default())
    .await
    .unwrap()
    .unwrap()
}

/// R of the plan: a retention campaign that starts today, set up by `by`.
async fn retention_campaign(t: &Booted, owner: i64, by: i64, segment: i64) -> Campaign {
    Campaign {
        name: "Win back".into(),
        code: "WINBACK".into(),
        purpose: Some(Purpose::Retention),
        marketing_channel: Some(MarketingChannel::Sms),
        owner_id: Some(owner),
        start_date: Some(today()),
        end_date: Some(today() + Duration::days(30)),
        segment_id: Some(segment),
        template_key: Some("promo.offer".into()),
        message_channel: Some("sms".into()),
        send_at: Some(whole_minute(now() + Duration::hours(2))),
        retention_set_by: Some(by),
        ..Campaign::default()
    }
    .store(t.db())
    .await
    .unwrap()
    .unwrap()
}

struct World {
    t: Booted,
    stage: Stage,
    /// A sets the retention fields.
    a: User,
    /// B decides and manages.
    b: User,
    r: Campaign,
}

async fn world() -> World {
    world_with(WlSales::new()).await
}

async fn world_with(sales: WlSales) -> World {
    let stage = Stage::default();
    let t = boot_configured(sales, stage.clone()).await;
    let a = t
        .user("Aisha", &[perm::CAMPAIGNS_VIEW, perm::CAMPAIGNS_MANAGE])
        .await;
    let b = t
        .user("Bilal", &[perm::CAMPAIGNS_VIEW, perm::CAMPAIGNS_MANAGE])
        .await;
    let owner = t.user("Owner", &[]).await;
    let s = lapsed(&t).await;
    let r = retention_campaign(&t, owner.id, a.id, s.id).await;
    World { t, stage, a, b, r }
}

impl World {
    /// R starts today: the daily run activates it.
    async fn start(&self) {
        let report = campaigns::run_daily(self.t.state(), today()).await.unwrap();
        assert_eq!((report.activated, report.failed), (1, 0));
    }

    async fn rows(&self) -> Vec<CampaignBroadcast> {
        CampaignBroadcast::of_campaign(self.t.db(), self.r.id)
            .await
            .unwrap()
    }

    async fn reload(&self) -> Campaign {
        Campaign::find(self.t.db(), self.r.id)
            .await
            .unwrap()
            .unwrap()
    }

    /// wl-notify rejected the broadcast with `comment`.
    async fn rejected(&self, broadcast: i64, comment: &str) {
        self.stage
            .gate
            .report(broadcast, report_of(broadcast, "cancelled", Some(comment)));
        self.t
            .state()
            .emit(BroadcastCancelled {
                broadcast: BroadcastId(broadcast),
                requester: campaign_subject(self.r.id),
                reason: BroadcastCancelReason::Rejected,
            })
            .await
            .unwrap();
    }

    /// wl-notify completed the broadcast.
    async fn completed(&self, broadcast: i64, recipients: i64) {
        self.t
            .state()
            .emit(BroadcastCompleted {
                broadcast: BroadcastId(broadcast),
                requester: campaign_subject(self.r.id),
                channel: NotifyChannel::Sms,
                recipients,
                excluded: BTreeMap::new(),
            })
            .await
            .unwrap();
    }
}

fn report_of(id: i64, status: &str, cancel_reason: Option<&str>) -> BroadcastReport {
    BroadcastReport {
        broadcast: BroadcastId(id),
        status: status.to_owned(),
        send_at: now() - Duration::days(10),
        estimate: 180,
        recipients: None,
        excluded: BTreeMap::new(),
        delivered: 0,
        opted_out: 0,
        cancel_reason: cancel_reason.map(str::to_owned),
        reached: Vec::new(),
    }
}

#[renox::test]
async fn activating_on_the_start_date_asks_for_one_broadcast_naming_the_user_who_set_the_fields() {
    let w = world().await;
    w.start().await;
    let asked = w.stage.gate.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].requested_by, w.a.id);
    assert_eq!(asked[0].requester, campaign_subject(w.r.id));
    assert_eq!(asked[0].type_key, "promo.offer");
    assert_eq!(asked[0].channel, NotifyChannel::Sms);
    assert_eq!(asked[0].segment.0, w.r.segment_id.unwrap());
    assert_eq!(asked[0].send_at, w.r.send_at.unwrap());
    let rows = w.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, BroadcastState::Requested);
    assert_eq!(rows[0].broadcast_id, Some(1));
    assert_eq!(rows[0].requested_by, w.a.id);

    // The pass run again, and asking while one is open, add nothing.
    let again = campaigns::run_daily(w.t.state(), today()).await.unwrap();
    assert_eq!(again.activated, 0);
    w.t.app.acting_as(&w.b);
    w.t.app
        .post(&format!("/campaigns/{}/broadcast", w.r.id), &[("x", "")])
        .await
        .assert_status(422);
    assert_eq!(w.stage.gate.asked().len(), 1);
}

#[renox::test]
async fn a_send_time_already_past_goes_at_once() {
    let w = world().await;
    let mut late = w.reload().await;
    late.start_date = Some(today() - Duration::days(1));
    late.send_at = Some(whole_minute(now() - Duration::hours(1)));
    late.store(w.t.db()).await.unwrap().unwrap();
    // `activate` refuses a send time before the start; the start is a day
    // before, so the past send time is inside the period.
    w.t.app.acting_as(&w.b);
    w.t.app
        .post(
            &format!("/campaigns/{}/transitions/activate", w.r.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    let asked = w.stage.gate.asked();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].send_at >= now() - Duration::minutes(1),
        "{}",
        asked[0].send_at
    );
}

#[renox::test]
async fn a_rejected_broadcast_shows_not_sent_and_is_asked_once_more_after_a_fix() {
    let w = world().await;
    w.start().await;
    w.rejected(1, "wrong audience").await;
    let rows = w.rows().await;
    assert_eq!(rows[0].status, BroadcastState::Cancelled);
    assert_eq!(rows[0].reason.as_deref(), Some("wrong audience"));
    assert_eq!(w.reload().await.status, CampaignStatus::Active);

    w.t.app.acting_as(&w.b);
    w.t.app
        .get(&format!("/campaigns/{}", w.r.id))
        .await
        .assert_ok()
        .assert_see("Not sent")
        .assert_see("wrong audience")
        .assert_see("Request the broadcast again");

    // B saves the campaign with another name: A stays the requester.
    let id = w.r.id.to_string();
    let seg = w.r.segment_id.unwrap().to_string();
    let owner = w.r.owner_id.unwrap().to_string();
    let start = w.r.start_date.unwrap().to_string();
    let end = w.r.end_date.unwrap().to_string();
    let send = form_time(w.r.send_at.unwrap());
    let form = |name: &str, template: &str| -> Vec<(String, String)> {
        vec![
            ("name".into(), name.into()),
            ("code".into(), "WINBACK".into()),
            ("purpose".into(), "retention".into()),
            ("marketing_channel".into(), "sms".into()),
            ("owner_id".into(), owner.clone()),
            ("start_date".into(), start.clone()),
            ("end_date".into(), end.clone()),
            ("segment_id".into(), seg.clone()),
            ("template_key".into(), template.into()),
            ("message_channel".into(), "sms".into()),
            ("send_at".into(), send.clone()),
        ]
    };
    let post = |fields: Vec<(String, String)>| {
        let t = &w.t;
        let path = format!("/campaigns/{id}");
        async move {
            let pairs: Vec<(&str, &str)> = fields
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            t.app.post(&path, &pairs).await.assert_status(303);
        }
    };
    post(form("Win back again", "promo.offer")).await;
    assert_eq!(w.reload().await.retention_set_by, Some(w.a.id));

    // A changes the template and asks again: A is the requester.
    w.t.app.acting_as(&w.a);
    post(form("Win back again", "promo.offer2")).await;
    let fixed = w.reload().await;
    assert_eq!(fixed.template_key.as_deref(), Some("promo.offer2"));
    assert_eq!(fixed.retention_set_by, Some(w.a.id));
    w.t.app.acting_as(&w.b);
    w.t.app
        .post(&format!("/campaigns/{}/broadcast", w.r.id), &[("x", "")])
        .await
        .assert_status(303);
    let asked = w.stage.gate.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1].type_key, "promo.offer2");
    assert_eq!(asked[1].requested_by, w.a.id);

    // It completes; a third request is refused, and so is a reopened campaign
    // that tries to send again.
    w.completed(2, 180).await;
    assert_eq!(w.rows().await[1].status, BroadcastState::Completed);
    w.t.app
        .post(&format!("/campaigns/{}/broadcast", w.r.id), &[("x", "")])
        .await
        .assert_status(422);
    w.t.app
        .post(
            &format!("/campaigns/{}/transitions/end", w.r.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    assert!(w.stage.gate.cancelled.lock().unwrap().is_empty());
    let later = (today() + Duration::days(40)).to_string();
    w.t.app
        .post(
            &format!("/campaigns/{}/reopen", w.r.id),
            &[("end_date", later.as_str())],
        )
        .await
        .assert_status(303);
    assert_eq!(w.reload().await.status, CampaignStatus::Active);
    assert_eq!(w.stage.gate.asked().len(), 2, "reopen sends nothing");
    assert_eq!(w.rows().await.len(), 2);
}

#[renox::test]
async fn a_refusal_of_the_notifier_keeps_the_campaign_active_with_the_reason() {
    let w = world().await;
    w.stage.gate.refuse(Some("the template has no SMS text"));
    w.start().await;
    let rows = w.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, BroadcastState::Failed);
    assert_eq!(rows[0].broadcast_id, None);
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("the template has no SMS text")
    );
    assert_eq!(w.reload().await.status, CampaignStatus::Active);
    w.t.app.acting_as(&w.b);
    w.t.app
        .get(&format!("/campaigns/{}", w.r.id))
        .await
        .assert_see("the template has no SMS text");

    // Fixed: asked again, the request goes through.
    w.stage.gate.refuse(None);
    w.t.app
        .post(&format!("/campaigns/{}/broadcast", w.r.id), &[("x", "")])
        .await
        .assert_status(303);
    let rows = w.rows().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].status, BroadcastState::Requested);
}

#[renox::test]
async fn ending_the_campaign_before_the_send_cancels_the_waiting_broadcast() {
    let w = world().await;
    w.start().await;
    w.t.app.acting_as(&w.b);
    w.t.app
        .post(
            &format!("/campaigns/{}/transitions/end", w.r.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(w.reload().await.status, CampaignStatus::Ended);
    assert_eq!(
        *w.stage.gate.cancelled.lock().unwrap(),
        vec![(1, retention::CAMPAIGN_ENDED.to_owned())]
    );
    let rows = w.rows().await;
    assert_eq!(rows[0].status, BroadcastState::Cancelled);
    assert_eq!(rows[0].reason.as_deref(), Some(retention::CAMPAIGN_ENDED));
}

#[renox::test]
async fn the_report_counts_the_recipients_who_bought_within_the_response_days() {
    let w = world().await;
    w.start().await;
    let sent = now() - Duration::days(10);
    // P1 and P2 got the message and bought 5 and 6 days ago; P3 got it and did
    // not buy; P4 bought, but its message was suppressed (the frequency cap),
    // so it is not among the recipients reached. P5 bought before the send.
    sale_at(&w.t, 1, now() - Duration::days(5), 15_000, "tee").await;
    sale_at(&w.t, 2, now() - Duration::days(6), 25_000, "tee").await;
    sale_at(&w.t, 4, now() - Duration::days(5), 40_000, "tee").await;
    sale_at(&w.t, 5, now() - Duration::days(12), 90_000, "tee").await;
    let mut late = report_of(1, "completed", None);
    late.send_at = sent;
    late.recipients = Some(180);
    late.delivered = 170;
    late.opted_out = 3;
    late.excluded = BTreeMap::from([
        ("no_consent".to_owned(), 120),
        ("frequency_cap".to_owned(), 20),
    ]);
    late.reached = vec![PartyId(1), PartyId(2), PartyId(3), PartyId(5)];
    w.stage.gate.report(1, late);
    w.completed(1, 180).await;
    // The campaign cost AED 300.
    let mut cost = CostEntry {
        campaign_id: w.r.id,
        entry_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        amount: 30_000,
        requested_by: w.a.id,
        status: CostStatus::Approved,
        ..CostEntry::default()
    };
    let mut tx = w.t.db().begin().await.unwrap();
    cost.insert_in_state(&mut tx, CostStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let campaign = w.reload().await;
    let report = retention::report(w.t.state(), &campaign).await.unwrap();
    assert_eq!(report.state, Some(BroadcastState::Completed));
    assert_eq!(report.recipients, Some(180));
    assert_eq!((report.delivered, report.opted_out), (170, 3));
    assert_eq!(report.excluded.get("no_consent"), Some(&120));
    assert_eq!(report.excluded.get("frequency_cap"), Some(&20));
    // P1 and P2 returned with AED 400 before VAT; P4 (cap) and P5 (before)
    // do not count.
    assert_eq!((report.returned, report.returned_value), (2, 40_000));
    assert_eq!(report.cost, 30_000);
    assert_eq!(report.response_days, 30);

    w.t.app.acting_as(&w.b);
    w.t.app
        .get(&format!("/campaigns/{}", w.r.id))
        .await
        .assert_ok()
        .assert_see("AED 400.00")
        .assert_see("AED 300.00")
        .assert_see("No marketing consent")
        .assert_see("120")
        .assert_see("Frequency cap");
}

#[renox::test]
async fn a_purchase_after_the_response_days_does_not_count() {
    let w = world_with(WlSales::new().retention_response_days(7)).await;
    w.start().await;
    let mut done = report_of(1, "completed", None);
    done.send_at = now() - Duration::days(20);
    done.reached = vec![PartyId(1), PartyId(2)];
    w.stage.gate.report(1, done);
    w.completed(1, 2).await;
    sale_at(&w.t, 1, now() - Duration::days(15), 10_000, "tee").await;
    sale_at(&w.t, 2, now() - Duration::days(5), 10_000, "tee").await;
    let report = retention::report(w.t.state(), &w.reload().await)
        .await
        .unwrap();
    assert_eq!((report.returned, report.returned_value), (1, 10_000));
    assert_eq!(report.response_days, 7);
}

/// Stops every campaign sending, as a missing permit would (05).
struct NoPermit;

impl CampaignGuard for NoPermit {
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _campaign: &'a Campaign,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>> {
        Box::pin(async { Ok(Some("advertising_permit".to_owned())) })
    }
}

#[renox::test]
async fn the_campaign_answers_wl_notify_only_while_it_is_active_and_the_vertical_agrees() {
    let w = world().await;
    let subject = campaign_subject(w.r.id);
    let check = |t: &Booted| {
        let subject = subject.clone();
        let state = t.state().clone();
        async move {
            CampaignRequesters
                .check(&state, &subject, now())
                .await
                .unwrap()
        }
    };
    // A draft is not active.
    assert_eq!(check(&w.t).await, RequesterCheck::Ended);
    w.start().await;
    assert_eq!(check(&w.t).await, RequesterCheck::Active);
    w.t.app.acting_as(&w.b);
    w.t.app
        .post(
            &format!("/campaigns/{}/transitions/end", w.r.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(check(&w.t).await, RequesterCheck::Ended);
    let other = campaign_subject(9_999);
    assert_eq!(
        CampaignRequesters
            .check(w.t.state(), &other, now())
            .await
            .unwrap(),
        RequesterCheck::Ended
    );

    // A guard of the vertical stops the send: the permit.
    let w = world_with(WlSales::new().campaign_guard(NoPermit)).await;
    w.start().await;
    assert_eq!(
        CampaignRequesters
            .check(w.t.state(), &campaign_subject(w.r.id), now())
            .await
            .unwrap(),
        RequesterCheck::GuardFailed("advertising_permit".to_owned())
    );
}
