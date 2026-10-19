//! Campaigns: the code rules, the machine, the daily run, the promo code
//! effects, the events and the owner's end notice (#138 part 4; AC2, AC3,
//! AC4, AC15, AC16 and the delete rules of AC14 that do not need cost
//! entries). A test cannot build an `AuthUser`, so the user transitions are
//! checked through their guards and effects.

mod support;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest, provide_notifier};
use wl_kernel::sales::{CampaignActivated, CampaignEnded};
use wl_sales::machine::{self, ReopenInput};
use wl_sales::{
    Campaign, CampaignRefused, CampaignStatus, LeadSource, LeadSourceRefused, MarketingChannel,
    PromoCode, Purpose, campaigns,
};

use support::{Booted, boot_with};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

#[derive(Clone, Default)]
struct Recorder {
    activated: Arc<Mutex<Vec<CampaignActivated>>>,
    ended: Arc<Mutex<Vec<CampaignEnded>>>,
    sent: Arc<Mutex<Vec<NotifyRequest>>>,
}

impl Notifier for Recorder {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(request);
            Ok(vec![MessageId(1)])
        })
    }
}

struct Recording(Recorder);

impl Module for Recording {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn register(&self, app: &mut Registry) {
        provide_notifier(app, self.0.clone());
        let log = self.0.activated.clone();
        app.listen(move |event: CampaignActivated, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.0.ended.clone();
        app.listen(move |event: CampaignEnded, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}

async fn booted() -> (Booted, Recorder, i64) {
    let recorder = Recorder::default();
    let t = boot_with(Recording(recorder.clone())).await;
    let owner = t.user("Owner", &[]).await.id;
    (t, recorder, owner)
}

/// C1 of the plan: `SPRING26`, Meta, 1-31 Mar 2026, AED 5,000.
fn c1(owner: i64) -> Campaign {
    Campaign {
        name: "Spring".into(),
        code: "spring26".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        owner_id: Some(owner),
        start_date: Some(day(3, 1)),
        end_date: Some(day(3, 31)),
        budget: Some(500_000),
        promo_code: Some("spring26".into()),
        ..Campaign::default()
    }
}

async fn stored(t: &Booted, campaign: Campaign) -> Campaign {
    campaign.store(t.db()).await.unwrap().unwrap()
}

async fn reload(t: &Booted, id: i64) -> Campaign {
    Campaign::find(t.db(), id).await.unwrap().unwrap()
}

#[renox::test]
async fn the_code_is_stored_upper_case_and_refused_when_taken_in_any_state() {
    let (t, _, owner) = booted().await;
    let first = stored(&t, c1(owner)).await;
    assert_eq!(first.code, "SPRING26");
    assert_eq!(first.status, CampaignStatus::Draft);

    let again = Campaign {
        code: " spring26 ".into(),
        promo_code: None,
        ..c1(owner)
    };
    assert_eq!(
        again.store(t.db()).await.unwrap().unwrap_err(),
        CampaignRefused::CodeTaken
    );
    // The same campaign keeps its code when saved again.
    let mut edited = reload(&t, first.id).await;
    edited.name = "Spring sale".into();
    let edited = stored(&t, edited).await;
    assert_eq!(edited.name, "Spring sale");

    // An ended campaign's code is taken too.
    Campaign::query()
        .where_(Campaign::ID.eq(first.id))
        .update(t.db(), &[("status", &"ended")])
        .await
        .unwrap();
    let other = Campaign {
        promo_code: None,
        ..c1(owner)
    };
    assert_eq!(
        other.store(t.db()).await.unwrap().unwrap_err(),
        CampaignRefused::CodeTaken
    );
    // The status of a saved campaign does not come from the caller.
    let mut ended = reload(&t, first.id).await;
    ended.status = CampaignStatus::Draft;
    let saved = stored(&t, ended).await;
    assert_eq!(saved.status, CampaignStatus::Ended);
}

#[renox::test]
async fn bad_codes_names_and_dates_are_refused_on_save() {
    let (t, _, owner) = booted().await;
    let refused = |campaign: Campaign| {
        let db = t.db().clone();
        async move { campaign.store(&db).await.unwrap().unwrap_err() }
    };
    let base = Campaign {
        promo_code: None,
        ..c1(owner)
    };
    assert_eq!(
        refused(Campaign {
            code: "x".into(),
            ..base.clone()
        })
        .await,
        CampaignRefused::BadCode
    );
    assert_eq!(
        refused(Campaign {
            code: "has space".into(),
            ..base.clone()
        })
        .await,
        CampaignRefused::BadCode
    );
    assert_eq!(
        refused(Campaign {
            name: "  ".into(),
            ..base.clone()
        })
        .await,
        CampaignRefused::NameRequired
    );
    assert_eq!(
        refused(Campaign {
            end_date: Some(day(2, 28)),
            ..base.clone()
        })
        .await,
        CampaignRefused::EndBeforeStart
    );
    assert_eq!(
        refused(Campaign {
            promo_code: Some("no way".into()),
            ..base
        })
        .await,
        CampaignRefused::BadPromoCode
    );
    assert_eq!(Campaign::query().count(t.db()).await.unwrap(), 0);
}

#[renox::test]
async fn a_promo_code_belongs_to_one_campaign() {
    let (t, _, owner) = booted().await;
    let first = stored(&t, c1(owner)).await;
    let promo = PromoCode::of_campaign(t.db(), first.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(promo.code, "SPRING26");
    let second = Campaign {
        code: "OTHER".into(),
        ..c1(owner)
    };
    assert_eq!(
        second.store(t.db()).await.unwrap().unwrap_err(),
        CampaignRefused::PromoCodeTaken
    );
    // Clearing the field removes the row.
    let mut cleared = reload(&t, first.id).await;
    cleared.promo_code = None;
    stored(&t, cleared).await;
    assert!(
        PromoCode::of_campaign(t.db(), first.id)
            .await
            .unwrap()
            .is_none()
    );
}

/// AC2, AC3 and the promo code of AC15.
#[renox::test]
async fn the_daily_run_activates_then_ends_c1_once() {
    let (t, recorder, owner) = booted().await;
    let c = stored(&t, c1(owner)).await;

    // Before the start: nothing.
    let report = campaigns::run_daily(t.state(), day(2, 28)).await.unwrap();
    assert_eq!(report, campaigns::Report::default());

    let report = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!((report.activated, report.ended, report.failed), (1, 0, 0));
    assert_eq!(reload(&t, c.id).await.status, CampaignStatus::Active);
    {
        let activated = recorder.activated.lock().unwrap();
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].campaign.0, c.id);
        assert_eq!(activated[0].purpose, "acquisition");
        assert_eq!(activated[0].marketing_channel, "meta");
        assert_eq!(activated[0].start_date, day(3, 1));
        assert_eq!(activated[0].end_date, Some(day(3, 31)));
        assert_eq!(activated[0].budget, Some(500_000));
    }
    let promo = PromoCode::of_campaign(t.db(), c.id).await.unwrap().unwrap();
    assert_eq!(promo.valid_to, Some(day(3, 31)));

    // A second run on the same day finds nothing and emits nothing.
    let again = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!(again, campaigns::Report::default());
    assert_eq!(recorder.activated.lock().unwrap().len(), 1);

    // The last day it still runs; the day after it ends.
    let on_last = campaigns::run_daily(t.state(), day(3, 31)).await.unwrap();
    assert_eq!(on_last, campaigns::Report::default());
    let report = campaigns::run_daily(t.state(), day(4, 1)).await.unwrap();
    assert_eq!((report.activated, report.ended), (0, 1));
    assert_eq!(reload(&t, c.id).await.status, CampaignStatus::Ended);
    assert_eq!(recorder.ended.lock().unwrap().len(), 1);
    let promo = PromoCode::of_campaign(t.db(), c.id).await.unwrap().unwrap();
    assert_eq!(promo.valid_to, Some(day(4, 1)));
    {
        let sent = recorder.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].type_key, "sales.campaign_ended");
        assert_eq!(sent[0].recipients.len(), 1);
        assert_eq!(sent[0].data["code"], "SPRING26");
    }
    let again = campaigns::run_daily(t.state(), day(4, 1)).await.unwrap();
    assert_eq!(again, campaigns::Report::default());
    assert_eq!(recorder.ended.lock().unwrap().len(), 1);
    assert_eq!(recorder.sent.lock().unwrap().len(), 1);
}

#[renox::test]
async fn a_run_late_activates_a_draft_in_its_period_but_not_one_past_its_end() {
    let (t, recorder, owner) = booted().await;
    let in_period = stored(&t, c1(owner)).await;
    let past = stored(
        &t,
        Campaign {
            code: "OLD".into(),
            promo_code: None,
            end_date: Some(day(3, 10)),
            ..c1(owner)
        },
    )
    .await;
    let report = campaigns::run_daily(t.state(), day(3, 15)).await.unwrap();
    assert_eq!((report.activated, report.ended, report.failed), (1, 0, 0));
    assert_eq!(
        reload(&t, in_period.id).await.status,
        CampaignStatus::Active
    );
    assert_eq!(reload(&t, past.id).await.status, CampaignStatus::Draft);
    assert_eq!(recorder.activated.lock().unwrap().len(), 1);
}

/// AC16 (the fields part): a retention campaign missing its fields stays a draft.
#[renox::test]
async fn a_draft_with_missing_fields_stays_a_draft_and_is_counted() {
    let (t, recorder, owner) = booted().await;
    let retention = stored(
        &t,
        Campaign {
            purpose: Some(Purpose::Retention),
            promo_code: None,
            ..c1(owner)
        },
    )
    .await;
    let report = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!((report.activated, report.failed), (0, 1));
    assert_eq!(reload(&t, retention.id).await.status, CampaignStatus::Draft);
    assert!(recorder.activated.lock().unwrap().is_empty());

    // With every field, and a send time inside the period, it activates.
    let mut full = reload(&t, retention.id).await;
    full.segment_id = Some(1);
    full.template_key = Some("promo".into());
    full.message_channel = Some("email".into());
    full.send_at = Some(day(3, 10).and_hms_opt(6, 0, 0).unwrap().and_utc());
    stored(&t, full).await;
    let report = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!((report.activated, report.failed), (1, 0));
}

/// AC16 (the send time part): a send time outside the period, or a missing
/// one, keeps a retention campaign a draft.
#[renox::test]
async fn a_send_time_outside_the_period_keeps_a_retention_campaign_a_draft() {
    let (t, recorder, owner) = booted().await;
    let retention = stored(
        &t,
        Campaign {
            purpose: Some(Purpose::Retention),
            promo_code: None,
            segment_id: Some(1),
            template_key: Some("promo".into()),
            message_channel: Some("email".into()),
            send_at: Some(day(4, 10).and_hms_opt(6, 0, 0).unwrap().and_utc()),
            ..c1(owner)
        },
    )
    .await;
    let report = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!((report.activated, report.failed), (0, 1), "after the end");
    assert_eq!(reload(&t, retention.id).await.status, CampaignStatus::Draft);

    let mut early = reload(&t, retention.id).await;
    early.send_at = Some(day(2, 10).and_hms_opt(6, 0, 0).unwrap().and_utc());
    stored(&t, early).await;
    let report = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!(
        (report.activated, report.failed),
        (0, 1),
        "before the start"
    );

    let mut none = reload(&t, retention.id).await;
    none.send_at = None;
    stored(&t, none).await;
    let report = campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!((report.activated, report.failed), (0, 1), "no send time");
    assert_eq!(reload(&t, retention.id).await.status, CampaignStatus::Draft);
    assert!(recorder.activated.lock().unwrap().is_empty());
}

fn reopen_ctx(end: NaiveDate, today: NaiveDate) -> TransitionCtx {
    TransitionCtx::new(None)
        .with_input(renox::serde_json::to_value(ReopenInput { end_date: end }).unwrap())
        .with_meta(machine::today_meta(today))
}

/// AC3 and AC15: reopen needs a future end date and makes the promo code
/// valid again, unless its uses are used up.
#[renox::test]
async fn reopen_takes_a_future_end_date_and_revives_the_promo_code() {
    let (t, _, owner) = booted().await;
    let c = stored(&t, c1(owner)).await;
    campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    campaigns::run_daily(t.state(), day(4, 1)).await.unwrap();
    let ended = reload(&t, c.id).await;
    assert_eq!(ended.status, CampaignStatus::Ended);

    let machine = Campaign::machine();
    let reopen = machine.find(machine::REOPEN).unwrap();
    let past = reopen
        .check_guards(&ended, &reopen_ctx(day(3, 31), day(4, 1)))
        .await;
    assert!(past.is_err(), "an end date in the past is refused");
    let today = reopen
        .check_guards(&ended, &reopen_ctx(day(4, 1), day(4, 1)))
        .await;
    assert!(today.is_err(), "today is not after today");
    let before_start = reopen
        .check_guards(&ended, &reopen_ctx(day(2, 1), day(1, 1)))
        .await;
    assert!(before_start.is_err());
    let no_input = reopen.check_guards(&ended, &TransitionCtx::new(None)).await;
    assert!(no_input.is_err());

    let ctx = reopen_ctx(day(4, 15), day(4, 1));
    reopen.check_guards(&ended, &ctx).await.unwrap();
    let mut tx = t.db().begin().await.unwrap();
    reopen.run_effects(&ended, &ctx, &mut tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(reload(&t, c.id).await.end_date, Some(day(4, 15)));
    let promo = PromoCode::of_campaign(t.db(), c.id).await.unwrap().unwrap();
    assert_eq!(promo.valid_to, Some(day(4, 15)));

    // A promo code whose uses are used up stays expired.
    PromoCode::query()
        .where_(PromoCode::CAMPAIGN_ID.eq(c.id))
        .update(
            t.db(),
            &[
                ("valid_to", &Some(day(4, 1))),
                ("max_uses", &Some(5_i64)),
                ("uses", &5_i64),
            ],
        )
        .await
        .unwrap();
    let ctx = reopen_ctx(day(4, 20), day(4, 1));
    let mut tx = t.db().begin().await.unwrap();
    reopen.run_effects(&ended, &ctx, &mut tx).await.unwrap();
    tx.commit().await.unwrap();
    let promo = PromoCode::of_campaign(t.db(), c.id).await.unwrap().unwrap();
    assert_eq!(promo.valid_to, Some(day(4, 1)));
}

#[renox::test]
async fn the_system_twins_refuse_the_wrong_day_and_the_user_ones_need_an_actor() {
    let (t, _, owner) = booted().await;
    let c = stored(&t, c1(owner)).await;
    // Before its start date `activate_on_start` is refused.
    let early = c
        .transition(machine::ACTIVATE_ON_START)
        .by_system()
        .meta(machine::today_meta(day(2, 28)))
        .run(t.state())
        .await;
    assert!(early.is_err());
    // The user transition does not run as the system.
    let manual = c
        .transition(machine::ACTIVATE)
        .by_system()
        .run(t.state())
        .await;
    assert!(manual.is_err());
    let c = reload(&t, c.id).await;
    assert_eq!(c.status, CampaignStatus::Draft);

    let on = c
        .transition(machine::ACTIVATE_ON_START)
        .by_system()
        .meta(machine::today_meta(day(3, 1)))
        .run(t.state())
        .await;
    assert!(on.is_ok());
    // Not yet past its end date: `end_on_end_date` is refused.
    let c = reload(&t, c.id).await;
    let early_end = c
        .transition(machine::END_ON_END_DATE)
        .by_system()
        .meta(machine::today_meta(day(3, 31)))
        .run(t.state())
        .await;
    assert!(early_end.is_err());
}

fn blocker<'a>(
    _tx: &'a mut Transaction,
    _id: i64,
) -> Pin<Box<dyn Future<Output = Result<Option<String>>> + Send + 'a>> {
    Box::pin(async {
        Ok(Some(
            "This campaign has cost entries; it cannot be deleted".to_owned(),
        ))
    })
}

/// AC4 (deleted code) and AC14 (the draft rules and the hook list).
#[renox::test]
async fn only_a_draft_nothing_refers_to_is_deleted_and_its_code_stays_taken() {
    let (t, _, owner) = booted().await;
    let c = stored(&t, c1(owner)).await;

    // A hook that objects keeps the draft.
    let refused = campaigns::delete_with(t.db(), c.id, &[blocker])
        .await
        .unwrap();
    assert_eq!(
        refused,
        Err(CampaignRefused::InUse(
            "This campaign has cost entries; it cannot be deleted".to_owned()
        ))
    );
    assert_eq!(Campaign::query().count(t.db()).await.unwrap(), 1);

    campaigns::delete(t.db(), c.id).await.unwrap().unwrap();
    assert_eq!(Campaign::query().count(t.db()).await.unwrap(), 0);
    assert!(
        PromoCode::of_campaign(t.db(), c.id)
            .await
            .unwrap()
            .is_none()
    );
    let again = Campaign {
        promo_code: None,
        ..c1(owner)
    };
    assert_eq!(
        again.store(t.db()).await.unwrap().unwrap_err(),
        CampaignRefused::CodeTaken
    );
    assert_eq!(
        campaigns::delete(t.db(), c.id).await.unwrap(),
        Err(CampaignRefused::NotFound)
    );

    // An active campaign cannot be deleted.
    let live = stored(
        &t,
        Campaign {
            code: "LIVE".into(),
            promo_code: None,
            ..c1(owner)
        },
    )
    .await;
    campaigns::run_daily(t.state(), day(3, 1)).await.unwrap();
    assert_eq!(
        campaigns::delete(t.db(), live.id).await.unwrap(),
        Err(CampaignRefused::NotDraft)
    );
}

#[renox::test]
async fn a_lead_source_a_campaign_names_cannot_be_deleted() {
    let (t, _, owner) = booted().await;
    let paid = LeadSource::by_code(t.db(), "paid_ads")
        .await
        .unwrap()
        .unwrap();
    stored(
        &t,
        Campaign {
            lead_source_id: Some(paid.id),
            promo_code: None,
            ..c1(owner)
        },
    )
    .await;
    assert_eq!(
        paid.delete_unused(t.db()).await.unwrap(),
        Err(LeadSourceRefused::InUse)
    );
}

#[renox::test]
async fn the_daily_task_is_scheduled_at_five_past_midnight_dubai() {
    let (t, _, _) = booted().await;
    t.app
        .kernel()
        .call(
            "sales:campaigns",
            vec!["--date".to_owned(), "2026-03-01".to_owned()],
        )
        .await
        .unwrap();
    assert_eq!(wl_sales::daily::TASK_AT, "00:05");
    assert_eq!(wl_sales::daily::TASK_ZONE, "Asia/Dubai");
}
