//! What a block does to each kind of agreement (wl-parties §5.6): the
//! acceptance checks AC1–AC8 of #36. The routes act as the user
//! (`acting_as`); test-only `booking` and `subscription` kinds with
//! in-memory sources, and a test vertical's overrides, stand for the owning
//! modules and the vertical. The kernel registry is process-wide, so the
//! kinds are registered once and read their data from a store each app
//! provides.

mod support;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, Once};

use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_kernel::block_effects::{
    self, AgreementFuture, AgreementKind, AgreementRef, AgreementSource, BlockEffect,
    BlockEffectOverride, Decision, RuleSource,
};
use wl_kernel::notify::{self, NotifyChannel, NotifyRecipient, NotifyRequest};
use wl_kernel::{NEUTRAL_BLOCK_REASON, PartyBlocked, PartyId, Subject};
use wl_notify::channels::ProviderFuture;
use wl_notify::{ChannelProvider, Message, MessageStatus, Outgoing, SendError, WlNotify};
use wl_parties::notify::NEUTRAL_TYPE;
use wl_parties::review_tasks::{ItemDecision, ReviewTask, ReviewTaskStatus};
use wl_parties::{WlParties, permissions as perm};

use support::{Booted, block_party, boot_custom, staff_in};

const AL_NOOR: &str = "Al Noor Trading LLC";
const MARINA_CLINIC: (&str, i64) = ("clinics", 1);
const MARINA_BRANCH: (&str, i64) = ("branches", 1);
const PAUSE_NOTICE: &str = "subscription.paused";

fn marina_clinic() -> Scope {
    Scope::new(MARINA_CLINIC.0, MARINA_CLINIC.1)
}

fn marina_branch() -> Scope {
    Scope::new(MARINA_BRANCH.0, MARINA_BRANCH.1)
}

/// One decision the test source carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    subject: i64,
    user: i64,
    decision: Decision,
    reason: &'static str,
}

/// What the owning modules keep, per app.
#[derive(Default)]
struct Store {
    /// Open bookings: `(id, party)`, all in the Marina clinic.
    bookings: Mutex<Vec<(i64, i64)>>,
    /// Booking ids the source refuses to decide.
    refused: Mutex<HashSet<i64>>,
    /// Every decision carried out.
    calls: Mutex<Vec<Call>>,
    /// Subscriptions: `(id, party)`, all in the Marina branch.
    subscriptions: Mutex<Vec<(i64, i64)>>,
    /// What the subscription listener was told by `effect_for`.
    seen: Mutex<Vec<block_effects::BlockEffectRule>>,
}

fn store_of(state: &AppState) -> Arc<Store> {
    match state.provided::<Arc<Store>>() {
        Some(store) => Arc::clone(&store),
        None => panic!("the test store is provided"),
    }
}

/// The test `booking` module: lists the party's bookings, carries a
/// decision out unless it refuses.
struct Bookings;

impl AgreementSource for Bookings {
    fn open_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> AgreementFuture<'a, Vec<AgreementRef>> {
        let store = store_of(state);
        Box::pin(async move {
            let bookings = store.bookings.lock().unwrap();
            Ok(bookings
                .iter()
                .filter(|(_, owner)| *owner == party.0)
                .map(|(id, _)| AgreementRef {
                    subject: Subject::new("clinic.booking", *id),
                    label: format!("Booking {id}"),
                    scope: Some(marina_clinic()),
                })
                .collect())
        })
    }

    fn decide<'a>(
        &'a self,
        state: &'a AppState,
        user: i64,
        subject: &'a Subject,
        decision: Decision,
    ) -> AgreementFuture<'a, ()> {
        let store = store_of(state);
        Box::pin(async move {
            if store.refused.lock().unwrap().contains(&subject.subject_id) {
                return Err(renox::Error::permanent_message("booking is checked in"));
            }
            store.calls.lock().unwrap().push(Call {
                subject: subject.subject_id,
                user,
                decision,
                reason: NEUTRAL_BLOCK_REASON,
            });
            Ok(())
        })
    }
}

/// The kinds the test modules register and the test vertical's overrides.
fn register_modules_and_vertical() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        AgreementKind::new("booking")
            .owner("clinic")
            .effect(BlockEffect::Review)
            .decider("branch_manager")
            .source(Bookings)
            .register();
        AgreementKind::new("subscription")
            .owner("invoicing")
            .effect(BlockEffect::Pause)
            .decider("subscriptions_manager")
            .register();
        BlockEffectOverride::kind("booking")
            .decider("clinic_manager")
            .register();
        BlockEffectOverride::kind("subscription")
            .decider("finance_manager")
            .register();
        BlockEffectOverride::kind("deployment")
            .effect(BlockEffect::Continue)
            .decider("managing_director")
            .register();
    });
}

/// A scripted e-mail provider that keeps what it was given.
#[derive(Clone, Default)]
struct Mailer(Arc<Mutex<Vec<Outgoing>>>);

impl ChannelProvider for Mailer {
    fn name(&self) -> &'static str {
        "test-mailer"
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
        let id = self.0.lock().unwrap().len();
        Box::pin(async move { Ok(format!("mail-{id}")) })
    }
}

/// The app with the test kinds, the store, the subscription module's
/// listener and, when `notify`, wl-notify with `mailer`.
async fn boot(notify_module: Option<Mailer>) -> (Booted, Arc<Store>) {
    register_modules_and_vertical();
    let store = Arc::new(Store::default());
    let provided = Arc::clone(&store);
    let t = boot_custom(WlParties::new(), true, move |app| {
        let app = app.provide(Arc::clone(&provided)).listen(
            |event: PartyBlocked, state: AppState| async move {
                // The subscription module reacts to the block itself.
                let store = store_of(&state);
                let owned = store
                    .subscriptions
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, party)| *party == event.party.0);
                if owned {
                    let rule = block_effects::effect_for("subscription", &marina_branch());
                    store.seen.lock().unwrap().push(rule);
                }
                Ok(())
            },
        );
        match notify_module {
            Some(mailer) => app.module(WlNotify::new().provider(mailer)),
            None => app,
        }
    })
    .await;
    (t, store)
}

/// [`boot`] with wl-notify, its types installed and the pause notice known.
async fn boot_notify() -> (Booted, Arc<Store>, Mailer) {
    notify::register_type(
        notify::NotificationTypeDef::new(PAUSE_NOTICE, "invoicing").template(
            NotifyChannel::Email,
            "en",
            Some("Your subscription is paused"),
            "Your subscription is paused.",
        ),
    );
    block_effects::mark_announces_pause(PAUSE_NOTICE);
    let mailer = Mailer::default();
    let (t, store) = boot(Some(mailer.clone())).await;
    t.app
        .kernel()
        .call("notify:install", Vec::<String>::new())
        .await
        .expect("notify:install runs");
    (t, store, mailer)
}

/// Marina Clinic's manager, three future bookings of Al Noor.
async fn clinic_with_three_bookings(t: &Booted, store: &Store) -> (User, i64) {
    let party = t.party(AL_NOOR).await;
    *store.bookings.lock().unwrap() = vec![(11, party.id), (12, party.id), (13, party.id)];
    let manager = staff_in(t, "Maha", "clinic_manager", &marina_clinic()).await;
    (manager, party.id)
}

async fn open_tasks(t: &Booted, party: i64) -> Vec<ReviewTask> {
    ReviewTask::open_for_party(t.db(), party).await.unwrap()
}

fn effect(kind: &str) -> (BlockEffect, Option<String>, RuleSource) {
    let rule = block_effects::effect_for(kind, &Scope::global());
    (rule.effect, rule.decider_role, rule.source)
}

/// AC1: the 12 kinds with their defaults, the vertical's override shown as
/// such; `parties.block` is needed.
#[renox::test]
async fn the_settings_list_shows_the_twelve_kinds_and_the_vertical() {
    let (t, _store) = boot(None).await;
    register_modules_and_vertical();
    let kinds = block_effects::kinds();
    let keys: Vec<&str> = kinds.iter().map(|k| k.key.as_str()).collect();
    for key in [
        "engagement",
        "subscription",
        "enrolment",
        "lease",
        "deployment",
        "project",
        "sales_order",
        "booking",
        "purchase_order",
        "service_contract",
        "money_out",
        "money_in",
    ] {
        assert!(keys.contains(&key), "`{key}` is listed");
    }
    // The kinds the vertical leaves alone keep the built-in default.
    for (key, kind_effect, decider) in [
        ("engagement", BlockEffect::Pause, "compliance"),
        ("enrolment", BlockEffect::Pause, "centre_manager"),
        ("lease", BlockEffect::Continue, "property_manager"),
        ("project", BlockEffect::Continue, "managing_director"),
        ("sales_order", BlockEffect::Pause, "sales_manager"),
        ("purchase_order", BlockEffect::Continue, "procurement"),
        ("service_contract", BlockEffect::Review, "facility_manager"),
        ("money_out", BlockEffect::Pause, "compliance"),
        ("money_in", BlockEffect::Continue, "compliance"),
    ] {
        assert_eq!(
            effect(key),
            (kind_effect, Some(decider.to_owned()), RuleSource::Default),
            "{key}"
        );
    }
    assert_eq!(
        effect("deployment"),
        (
            BlockEffect::Continue,
            Some("managing_director".to_owned()),
            RuleSource::Vertical
        )
    );

    let manager = t.user_a().await;
    t.app.acting_as(&manager);
    let page = t.app.get("/parties/block-effects").await;
    page.assert_ok()
        .assert_see("Deployment")
        .assert_see("Continue")
        .assert_see("managing_director")
        .assert_see("Vertical")
        .assert_see("clinic_manager")
        .assert_see("finance_manager")
        .assert_see("Default");

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app.get("/parties/block-effects").await.assert_forbidden();
}

/// AC2: three bookings in Marina Clinic give one task for the clinic
/// manager with three pending items, the reason in the header and one bell.
#[renox::test]
async fn a_block_opens_one_review_task_for_the_clinic_manager() {
    let (t, store) = boot(None).await;
    let (manager, party) = clinic_with_three_bookings(&t, &store).await;
    let outsider = staff_in(&t, "Omar", "front_desk", &marina_clinic()).await;
    let other_clinic = staff_in(
        &t,
        "Olga",
        "clinic_manager_elsewhere",
        &Scope::new("clinics", 2),
    )
    .await;
    let a = t.user_a().await;
    let al_noor = t.party(AL_NOOR).await;
    block_party(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;

    let tasks = open_tasks(&t, party).await;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.agreement_kind, "booking");
    assert_eq!(task.scope(), marina_clinic());
    assert_eq!(task.decider_role.as_deref(), Some("clinic_manager"));
    assert_eq!(task.display_reason, "non_payment");
    let items = task.items(t.db()).await.unwrap();
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.decision == ItemDecision::Pending));

    t.app.acting_as(&manager);
    let list = t.app.get("/parties/review-tasks").await;
    list.assert_ok()
        .assert_see(&format!("/parties/review-tasks/{}", task.id));
    assert!(
        list.text().contains("On hold \u{2014} Non-payment")
            || list.text().contains("On hold &mdash; Non-payment")
            || list.text().contains("On hold &#8212; Non-payment"),
        "the header names the reason: {}",
        list.text()
    );
    let page = t
        .app
        .get(&format!("/parties/review-tasks/{}", task.id))
        .await;
    page.assert_ok();
    for decision in ["pause", "continue", "end"] {
        assert_eq!(
            page.text()
                .matches(&format!("value=\"{decision}\""))
                .count(),
            3,
            "`{decision}` is offered on each item"
        );
    }

    let bells = manager.unread_notifications(t.db()).await.unwrap();
    assert_eq!(bells.len(), 1);
    assert_eq!(
        bells[0].message().expect("a message").title,
        "A party is on hold: 3 agreements to review"
    );
    // Not the clinic manager: no bell, no task, no page.
    for user in [&outsider, &other_clinic] {
        assert!(user.unread_notifications(t.db()).await.unwrap().is_empty());
        t.app.acting_as(user);
        t.app
            .get(&format!("/parties/review-tasks/{}", task.id))
            .await
            .assert_forbidden();
        assert!(
            !t.app
                .get("/parties/review-tasks")
                .await
                .text()
                .contains(&format!("/parties/review-tasks/{}", task.id))
        );
    }
}

/// AC3: a sanctions block shows "On hold" and nothing else.
#[renox::test]
async fn a_sanctions_block_leaves_only_on_hold_on_the_task() {
    let (t, store) = boot(None).await;
    let (manager, party) = clinic_with_three_bookings(&t, &store).await;
    let mlro = t.mlro().await;
    let al_noor = t.party(AL_NOOR).await;
    block_party(&t, &mlro, &al_noor, "sanctions", "List match on the owner").await;

    let tasks = open_tasks(&t, party).await;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.display_reason, "on_hold");
    let stored = format!(
        "{task:?} {:?} {:?}",
        task.items(t.db()).await.unwrap(),
        task.notes(t.db()).await.unwrap()
    );
    assert!(!stored.to_lowercase().contains("sanction"));

    t.app.acting_as(&manager);
    let list = t.app.get("/parties/review-tasks").await;
    list.assert_ok().assert_see("On hold");
    let page = t
        .app
        .get(&format!("/parties/review-tasks/{}", task.id))
        .await;
    page.assert_ok().assert_see("On hold");
    for text in [list.text(), page.text()] {
        assert!(!text.to_lowercase().contains("sanction"));
        assert!(!text.contains("Non-payment"));
    }
    let bells = manager.unread_notifications(t.db()).await.unwrap();
    assert_eq!(bells.len(), 1);
    let shown = format!("{:?}", bells[0].message().expect("a message"));
    assert!(!shown.to_lowercase().contains("sanction"));
    assert!(!shown.contains("hold_reason"));
}

/// AC4: a second `PartyBlocked` opens no second task while one is open.
#[renox::test]
async fn a_second_block_opens_no_second_task() {
    let (t, store) = boot(None).await;
    let (manager, party) = clinic_with_three_bookings(&t, &store).await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    let al_noor = t.party(AL_NOOR).await;
    block_party(&t, &a, &al_noor, "non_payment", "first").await;
    assert_eq!(open_tasks(&t, party).await.len(), 1);

    t.app.acting_as(&b);
    t.app
        .htmx()
        .post_multipart(
            &format!("/parties/{party}/unblock"),
            &[("note", "paid")],
            &[],
        )
        .await
        .assert_ok();
    block_party(&t, &b, &al_noor, "non_payment", "second").await;

    assert_eq!(t.blocked().len(), 2);
    assert_eq!(
        wl_parties::BlockRecord::for_party(t.db(), party)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(open_tasks(&t, party).await.len(), 1);
    assert_eq!(
        ReviewTask::query().count(t.db()).await.unwrap(),
        1,
        "no task of any status was added"
    );
    assert_eq!(manager.unread_notifications(t.db()).await.unwrap().len(), 1);
}

/// AC5: pause, continue and end are carried out once each through the
/// source, stored with user and time, and close the task; a refusal and a
/// user without the role record nothing.
#[renox::test]
async fn the_clinic_manager_decides_each_agreement() {
    let (t, store) = boot(None).await;
    let (manager, party) = clinic_with_three_bookings(&t, &store).await;
    let a = t.user_a().await;
    let al_noor = t.party(AL_NOOR).await;
    block_party(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;
    let task = open_tasks(&t, party).await.remove(0);
    let items = task.items(t.db()).await.unwrap();
    let decide = |item: i64, decision: &'static str| {
        let path = format!("/parties/review-tasks/{}/items/{item}", task.id);
        let t = &t;
        async move { t.app.htmx().post(&path, &[("decision", decision)]).await }
    };

    // No agreement is ended before a decision.
    assert!(store.calls.lock().unwrap().is_empty());

    // Without the decider role: 403 and nothing recorded.
    let outsider = staff_in(&t, "Omar", "front_desk", &marina_clinic()).await;
    t.app.acting_as(&outsider);
    decide(items[0].id, "end").await.assert_forbidden();
    assert!(store.calls.lock().unwrap().is_empty());
    assert!(
        task.items(t.db())
            .await
            .unwrap()
            .iter()
            .all(|i| i.decision == ItemDecision::Pending)
    );

    t.app.acting_as(&manager);
    // The source refuses: the item stays pending, the task open.
    store.refused.lock().unwrap().insert(11);
    decide(items[0].id, "end").await.assert_ok();
    let after = task.items(t.db()).await.unwrap();
    assert_eq!(after[0].decision, ItemDecision::Pending);
    assert_eq!(after[0].decided_by, None);
    assert!(store.calls.lock().unwrap().is_empty());
    store.refused.lock().unwrap().clear();

    // A decision that is not one of the three is refused.
    decide(items[0].id, "delete").await.assert_status(422);

    decide(items[0].id, "pause").await.assert_ok();
    decide(items[1].id, "continue").await.assert_ok();
    assert_eq!(
        ReviewTask::find(t.db(), task.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ReviewTaskStatus::Open
    );
    decide(items[2].id, "end").await.assert_ok();

    assert_eq!(
        *store.calls.lock().unwrap(),
        [
            Call {
                subject: 11,
                user: manager.id,
                decision: Decision::Pause,
                reason: NEUTRAL_BLOCK_REASON
            },
            Call {
                subject: 12,
                user: manager.id,
                decision: Decision::Continue,
                reason: NEUTRAL_BLOCK_REASON
            },
            Call {
                subject: 13,
                user: manager.id,
                decision: Decision::End,
                reason: NEUTRAL_BLOCK_REASON
            },
        ]
    );
    let done = task.items(t.db()).await.unwrap();
    assert_eq!(
        done.iter().map(|i| i.decision).collect::<Vec<_>>(),
        [
            ItemDecision::Pause,
            ItemDecision::Continue,
            ItemDecision::End
        ]
    );
    assert!(
        done.iter()
            .all(|i| i.decided_by == Some(manager.id) && i.decided_at.is_some())
    );
    assert_eq!(
        ReviewTask::find(t.db(), task.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ReviewTaskStatus::Closed
    );
    assert!(open_tasks(&t, party).await.is_empty());

    // A decided item is not carried out a second time.
    decide(items[2].id, "end").await.assert_ok();
    assert_eq!(store.calls.lock().unwrap().len(), 3);
}

/// AC6: a module that pauses on its own gets its rule from the registry
/// (the vertical's decider) and no review task is opened for it.
#[renox::test]
async fn a_pause_kind_is_the_modules_own_and_opens_no_task() {
    let (t, store) = boot(None).await;
    let party = t.party(AL_NOOR).await;
    *store.subscriptions.lock().unwrap() = vec![(21, party.id)];
    let a = t.user_a().await;
    block_party(&t, &a, &party, "non_payment", "3 bounced cheques").await;

    let seen = store.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].kind, "subscription");
    assert_eq!(seen[0].effect, BlockEffect::Pause);
    assert_eq!(seen[0].decider_role.as_deref(), Some("finance_manager"));
    assert_eq!(seen[0].scope, marina_branch());
    assert_eq!(seen[0].source, RuleSource::Vertical);
    assert!(open_tasks(&t, party.id).await.is_empty());
    assert_eq!(ReviewTask::query().count(t.db()).await.unwrap(), 0);

    let deployment = block_effects::effect_for("deployment", &marina_branch());
    assert_eq!(deployment.effect, BlockEffect::Continue);
    assert_eq!(
        deployment.decider_role.as_deref(),
        Some("managing_director")
    );
}

/// AC7: a kind nobody registered is reviewed, with no decider.
#[renox::test]
async fn an_unregistered_kind_is_reviewed_without_a_decider() {
    register_modules_and_vertical();
    let rule = block_effects::effect_for("unregistered_kind", &marina_branch());
    assert_eq!(rule.effect, BlockEffect::Review);
    assert_eq!(rule.decider_role, None);
    assert_eq!(rule.source, RuleSource::Default);
}

/// AC8: nothing is queued to the party by the block; a pause notice to the
/// blocked party is suppressed; the decider sends the neutral message.
#[renox::test]
async fn nothing_reaches_the_party_unless_the_decider_sends_the_neutral_message() {
    let (t, store, mailer) = boot_notify().await;
    let (manager, party) = clinic_with_three_bookings(&t, &store).await;
    *store.subscriptions.lock().unwrap() = vec![(21, party)];
    let a = t.user_a().await;
    let al_noor = t.party(AL_NOOR).await;
    block_party(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;

    t.app.run_all_jobs().await;
    assert!(mailer.0.lock().unwrap().is_empty());
    let to_party = || Message::query().where_(Message::PARTY_ID.eq(party));
    assert_eq!(to_party().count(t.db()).await.unwrap(), 0);

    // A type that announces a pause ends `suppressed`.
    let request = NotifyRequest::new(PAUSE_NOTICE, Subject::new("invoicing.subscription", 21))
        .to(NotifyRecipient::Party { id: PartyId(party) });
    let state = t.app.state();
    let ids = notify::notifier(state)
        .notify(state, request)
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    let message = Message::find(t.db(), ids[0].0).await.unwrap().unwrap();
    assert_eq!(message.status, MessageStatus::Suppressed);
    assert_eq!(message.suppress_reason.as_deref(), Some("party_on_hold"));
    t.app.run_all_jobs().await;
    assert!(mailer.0.lock().unwrap().is_empty());

    // The type is registered with both languages and never announces.
    let def = notify::notification_type(NEUTRAL_TYPE).expect("registered");
    for language in ["en", "ar"] {
        assert!(def.templates.iter().any(|tpl| tpl.language == language));
    }
    assert!(!block_effects::announces_pause(NEUTRAL_TYPE));

    // The decider's "Send neutral message" queues exactly one.
    let task = open_tasks(&t, party).await.remove(0);
    t.app.acting_as(&manager);
    let path = format!("/parties/review-tasks/{}/neutral-message", task.id);
    t.app.htmx().post(&path, &[]).await.assert_ok();
    let neutral = Message::query()
        .where_(Message::PARTY_ID.eq(party))
        .where_(Message::TYPE_KEY.eq(NEUTRAL_TYPE))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(neutral.len(), 1);
    assert_eq!(task.notes(t.db()).await.unwrap().len(), 1);
    t.app.run_all_jobs().await;
    let sent = mailer.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let text = format!("{} {}", sent[0].body, sent[0].title).to_lowercase();
    for word in [
        "non_payment",
        "non-payment",
        "payment",
        "sanction",
        "cheque",
    ] {
        assert!(!text.contains(word), "the message says `{word}`");
    }

    // Someone without the decider role cannot send it.
    let outsider = staff_in(&t, "Omar", "front_desk", &marina_clinic()).await;
    t.app.acting_as(&outsider);
    t.app.htmx().post(&path, &[]).await.assert_forbidden();
    assert_eq!(to_party().count(t.db()).await.unwrap(), 2);
}
