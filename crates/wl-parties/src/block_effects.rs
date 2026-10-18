//! What a block does to a party's agreements (wl-parties §5.6, issue #36).
//!
//! `PartyBlocked` ends nothing by itself. For every agreement kind whose
//! owning module registered an [`AgreementSource`] and whose block effect is
//! `review` ([`wl_kernel::block_effects`]), the listener lists the party's
//! open agreements, groups them by scope and opens one [`ReviewTask`] per
//! (party, kind, scope) with one item per agreement. A second `PartyBlocked`
//! opens no second task while one is open. The users holding the decider
//! role in the task's scope get a neutral in-app notice (never the party).
//!
//! Kinds with `pause` or `continue` get no task: their owning module reacts
//! to `PartyBlocked` itself. The task keeps only the event's display reason,
//! so nothing restricted reaches it (`PTY.ACL.02`).

use std::collections::BTreeMap;

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::Value;
use wl_kernel::block_effects::{self, AgreementRef, AgreementSource, BlockEffect, KindInfo};
use wl_kernel::{NEUTRAL_BLOCK_REASON, PartyBlocked, PartyId};

use crate::reasons::BlockReasons;
use crate::review_tasks::{ReviewTask, ReviewTaskItem, ReviewTaskStatus};
use crate::texts;

/// The kind of the in-app notice to the decider role.
pub const REVIEW_NOTICE_KIND: &str = "party-on-hold-review";

/// The header of a review task: "On hold" when `display_reason` is
/// `on_hold` (or blank, or restricted), else "On hold — <reason label>"
/// (#34 A9).
pub fn header(lang: &Lang, reasons: &BlockReasons, display_reason: &str) -> String {
    let code = display_reason.trim();
    if code.is_empty() || code == NEUTRAL_BLOCK_REASON || reasons.is_restricted(code) {
        return texts::t(lang, "wl_parties.review.on_hold");
    }
    let key = format!("wl_parties.reason.{code}");
    let label = texts::t(lang, &key);
    // A vertical's code without a text shows as the code.
    let label = if label == key { code.to_owned() } else { label };
    texts::t_with(
        lang,
        "wl_parties.review.on_hold_reason",
        &[("reason", &label)],
    )
}

/// The scope key stored on a task: `(kind, id)`, `None` when global.
type ScopeKey = Option<(String, i64)>;

/// The scope as stored on a task.
fn scope_key(scope: Option<&Scope>) -> Result<ScopeKey> {
    let Some(scope) = scope.filter(|scope| !scope.is_global()) else {
        return Ok(None);
    };
    let id = scope.id().parse::<i64>().map_err(|_| {
        renox::Error::permanent_message(format!(
            "scope `{scope}` of an agreement has no numeric id"
        ))
    })?;
    Ok(Some((scope.kind().to_owned(), id)))
}

/// The scope a stored key stands for.
fn scope_of(key: &ScopeKey) -> Scope {
    match key {
        Some((kind, id)) => Scope::new(kind, id),
        None => Scope::global(),
    }
}

/// The agreements grouped by scope, in scope order, each group in the order
/// the source listed them.
fn group_by_scope(refs: Vec<AgreementRef>) -> Result<BTreeMap<ScopeKey, Vec<AgreementRef>>> {
    let mut groups: BTreeMap<ScopeKey, Vec<AgreementRef>> = BTreeMap::new();
    for agreement in refs {
        groups
            .entry(scope_key(agreement.scope.as_ref())?)
            .or_default()
            .push(agreement);
    }
    Ok(groups)
}

/// Opens the review tasks for a blocked `party` (see the module doc) and
/// returns the ones newly opened. `display_reason` must be a display code.
///
/// A failing source, an agreement with an unusable scope or a failed notice
/// does not stop the other kinds; the first such error is returned after the
/// rest ran.
///
/// # Errors
///
/// A database error, or the first source or notice error.
pub async fn open_review_tasks(
    state: &AppState,
    party: PartyId,
    display_reason: &str,
) -> Result<Vec<ReviewTask>> {
    open_review_tasks_for(state, &block_effects::kinds(), party, display_reason).await
}

/// [`open_review_tasks`] limited to `kinds`.
async fn open_review_tasks_for(
    state: &AppState,
    kinds: &[KindInfo],
    party: PartyId,
    display_reason: &str,
) -> Result<Vec<ReviewTask>> {
    let mut opened = Vec::new();
    let mut failure = None;
    for info in kinds {
        // The deployment-level effect decides whether to ask the source at
        // all; the effect in each scope below decides the task.
        if !info.has_source || info.effect != BlockEffect::Review {
            continue;
        }
        let Some(source) = block_effects::source_for(&info.key) else {
            continue;
        };
        let groups = match list_groups(source.as_ref(), state, party).await {
            Ok(groups) => groups,
            Err(error) => {
                failure.get_or_insert(error);
                continue;
            }
        };
        for (key, refs) in groups {
            let scope = scope_of(&key);
            let rule = block_effects::effect_for(&info.key, &scope);
            if rule.effect != BlockEffect::Review {
                continue;
            }
            let task = open_task(
                state,
                party,
                &info.key,
                &key,
                rule.decider_role.as_deref(),
                display_reason,
                &refs,
            )
            .await;
            match task {
                Ok(Some(task)) => {
                    if let Err(error) = notify_deciders(state, &task, &scope, refs.len()).await {
                        failure.get_or_insert(error);
                    }
                    opened.push(task);
                }
                Ok(None) => {}
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
    }
    failure.map_or(Ok(opened), Err)
}

/// The party's open agreements from `source`, grouped by scope.
async fn list_groups(
    source: &dyn AgreementSource,
    state: &AppState,
    party: PartyId,
) -> Result<BTreeMap<ScopeKey, Vec<AgreementRef>>> {
    group_by_scope(source.open_for(state, party).await?)
}

/// Stores the task and its items in one transaction, unless an open task for
/// (party, kind, scope) exists already.
async fn open_task(
    state: &AppState,
    party: PartyId,
    kind: &str,
    key: &ScopeKey,
    decider_role: Option<&str>,
    display_reason: &str,
    refs: &[AgreementRef],
) -> Result<Option<ReviewTask>> {
    let mut tx = state.db.begin_immediate().await?;
    let scope_kind = key.as_ref().map(|(kind, _)| kind.as_str());
    let scope_id = key.as_ref().map(|(_, id)| *id);
    let existing = ReviewTask::query()
        .where_(ReviewTask::PARTY_ID.eq(party.0))
        .where_(ReviewTask::AGREEMENT_KIND.eq(kind))
        .where_(ReviewTask::STATUS.eq(ReviewTaskStatus::Open))
        .get(&mut tx)
        .await?;
    if existing
        .iter()
        .any(|task| task.scope_kind.as_deref() == scope_kind && task.scope_id == scope_id)
    {
        return Ok(None);
    }
    let mut task = ReviewTask::new(
        party.0,
        kind,
        scope_kind.zip(scope_id),
        decider_role,
        display_reason,
    );
    task.insert(&mut tx).await?;
    for agreement in refs {
        let mut item = ReviewTaskItem::pending(
            task.id,
            &agreement.subject.subject_type,
            agreement.subject.subject_id,
            &agreement.label,
        );
        item.insert(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(Some(task))
}

/// Tells the staff holding the task's decider role in `scope` that
/// agreements wait for review. The task is already stored; a task with no
/// decider role tells no one.
async fn notify_deciders(
    state: &AppState,
    task: &ReviewTask,
    scope: &Scope,
    count: usize,
) -> Result {
    let Some(role) = task.decider_role.as_deref() else {
        return Ok(());
    };
    let notice = ReviewNotice {
        task: task.id,
        count,
    };
    for user in users_with_role_in(&state.db, role, scope).await? {
        state.notify(&user, &notice).await?;
    }
    Ok(())
}

/// The neutral bell entry: how many agreements wait. It names no reason and
/// no party.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewNotice {
    task: i64,
    count: usize,
}

impl Notification for ReviewNotice {
    fn kind(&self) -> &'static str {
        REVIEW_NOTICE_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, state: &AppState) -> Result<Value> {
        let lang = state.current_lang();
        let title = if self.count == 1 {
            texts::t(&lang, "wl_parties.review.notice_title_one")
        } else {
            texts::t_with(
                &lang,
                "wl_parties.review.notice_title",
                &[("count", &self.count)],
            )
        };
        Ok(DatabaseMessage::info(title)
            .body(texts::t(&lang, "wl_parties.review.notice_body"))
            .with("review_task", self.task)
            .into())
    }
}

/// Opens the review tasks when `PartyBlocked` arrives.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: PartyBlocked, state: AppState| async move {
        open_review_tasks(&state, event.party, &event.display_reason)
            .await
            .map(|_| ())
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use renox::state_machine::StateMachine;
    use renox::testing::TestApp;
    use wl_kernel::block_effects::{AgreementFuture, AgreementKind, BlockEffectOverride, Decision};
    use wl_kernel::{ClientConfig, Subject};

    use crate::WlParties;
    use crate::party::Party;

    const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
default = true
"#;

    /// A source with a fixed list of open agreements.
    struct Fake(Vec<AgreementRef>);

    impl AgreementSource for Fake {
        fn open_for<'a>(
            &'a self,
            _state: &'a AppState,
            _party: PartyId,
        ) -> AgreementFuture<'a, Vec<AgreementRef>> {
            Box::pin(async move { Ok(self.0.clone()) })
        }

        fn decide<'a>(
            &'a self,
            _state: &'a AppState,
            _user: i64,
            _subject: &'a Subject,
            _decision: Decision,
        ) -> AgreementFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    /// A source that always fails.
    struct Broken;

    impl AgreementSource for Broken {
        fn open_for<'a>(
            &'a self,
            _state: &'a AppState,
            _party: PartyId,
        ) -> AgreementFuture<'a, Vec<AgreementRef>> {
            Box::pin(async { Err(renox::Error::permanent_message("source down")) })
        }

        fn decide<'a>(
            &'a self,
            _state: &'a AppState,
            _user: i64,
            _subject: &'a Subject,
            _decision: Decision,
        ) -> AgreementFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    fn agreement(kind: &str, id: i64, scope: Option<Scope>) -> AgreementRef {
        AgreementRef {
            subject: Subject::new(&format!("test.{kind}"), id),
            label: format!("{kind} {id}"),
            scope,
        }
    }

    /// Registers `key` as a kind with the effect, decider and a fake source.
    fn register_kind(key: &str, effect: BlockEffect, decider: &str, refs: Vec<AgreementRef>) {
        AgreementKind::new(key)
            .owner("test")
            .effect(effect)
            .decider(decider)
            .source(Fake(refs))
            .register();
    }

    fn info(key: &str) -> Vec<KindInfo> {
        block_effects::kinds()
            .into_iter()
            .filter(|kind| kind.key == key)
            .collect()
    }

    fn infos(keys: &[&str]) -> Vec<KindInfo> {
        keys.iter().flat_map(|key| info(key)).collect()
    }

    /// An app with wl-parties and the demo parties.
    pub(crate) async fn boot() -> (TestApp, Party, tempfile::TempDir) {
        let views = tempfile::tempdir().expect("a temporary views directory");
        let layout = views.path().join("layouts/app.html");
        std::fs::create_dir_all(layout.parent().unwrap()).unwrap();
        std::fs::write(
            &layout,
            "<html><body><rx-outlet name=\"content\"/></body></html>",
        )
        .unwrap();
        let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
        let app = App::new()
            .module(renox::auth::Auth::new())
            .module(renox::auth::Permissions)
            .module(WlParties::new());
        let app = wl_kernel::apply(app, config).expect("the kernel applies");
        let dir = views.path().to_path_buf();
        let app = TestApp::with_config(app, |c| c.views_path = dir).await;
        app.kernel()
            .call("wl:install", Vec::<String>::new())
            .await
            .expect("wl:install runs");
        crate::seed::run(app.state()).await.expect("seed");
        let party = Party::query()
            .first(app.db())
            .await
            .unwrap()
            .expect("a party");
        (app, party, views)
    }

    async fn staff(app: &TestApp, name: &str, role: &str, scope: &Scope) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(app.db(), name, &email, "password123")
            .await
            .unwrap();
        renox::auth::permissions::define_role(app.db(), role, &["parties.view"])
            .await
            .unwrap();
        user.assign_role_in(app.db(), role, scope).await.unwrap();
        user
    }

    #[renox::test]
    async fn the_header_names_the_reason_unless_it_is_on_hold() {
        let (app, _party, _views) = boot().await;
        let lang = app.state().current_lang();
        let reasons = BlockReasons::default();
        assert_eq!(header(&lang, &reasons, "on_hold"), "On hold");
        assert_eq!(header(&lang, &reasons, ""), "On hold");
        assert_eq!(
            header(&lang, &reasons, "non_payment"),
            "On hold \u{2014} Non-payment"
        );
        // A restricted code never shows, even if it reached the task.
        assert_eq!(header(&lang, &reasons, "sanctions"), "On hold");
        assert_eq!(header(&lang, &reasons, "aml_concern"), "On hold");
        // An unknown code counts as restricted.
        assert_eq!(header(&lang, &reasons, "tax_dispute"), "On hold");
        // A vertical's unrestricted code without a text shows as the code.
        let mut vertical = BlockReasons::default();
        vertical.add("tax_dispute", false);
        assert_eq!(
            header(&lang, &vertical, "tax_dispute"),
            "On hold \u{2014} tax_dispute"
        );
    }

    #[test]
    fn agreements_group_by_scope_and_global_has_no_scope() {
        let marina = Scope::new("branches", 7);
        let refs = vec![
            agreement("booking", 1, Some(marina.clone())),
            agreement("booking", 2, None),
            agreement("booking", 3, Some(marina.clone())),
            agreement("booking", 4, Some(Scope::global())),
        ];
        let groups = group_by_scope(refs).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[&None].len(), 2);
        assert_eq!(groups[&Some(("branches".to_owned(), 7))].len(), 2);
        assert_eq!(scope_of(&None), Scope::global());
        assert_eq!(scope_of(&Some(("branches".to_owned(), 7))), marina);
        // A scope id that is not a number cannot be stored on a task.
        let odd = vec![agreement("booking", 1, Some(Scope::new("branches", "x")))];
        assert!(group_by_scope(odd).is_err());
    }

    #[renox::test]
    async fn a_review_kind_gets_one_task_per_scope_with_one_item_per_agreement() {
        let (app, party, _views) = boot().await;
        let marina = Scope::new("test_clinics", 1);
        let jlt = Scope::new("test_clinics", 2);
        register_kind(
            "test_scoped",
            BlockEffect::Review,
            "test_scoped_manager",
            vec![
                agreement("scoped", 1, Some(marina.clone())),
                agreement("scoped", 2, Some(marina.clone())),
                agreement("scoped", 3, Some(marina.clone())),
                agreement("scoped", 4, Some(jlt)),
                agreement("scoped", 5, None),
            ],
        );
        let manager = staff(&app, "Maha", "test_scoped_manager", &marina).await;
        let outsider = staff(&app, "Omar", "test_scoped_other", &marina).await;

        let opened = open_review_tasks_for(
            app.state(),
            &info("test_scoped"),
            PartyId(party.id),
            "non_payment",
        )
        .await
        .unwrap();
        assert_eq!(opened.len(), 3);
        let tasks = ReviewTask::open_for_party(app.db(), party.id)
            .await
            .unwrap();
        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().all(|t| t.agreement_kind == "test_scoped"
            && t.display_reason == "non_payment"
            && t.status == ReviewTaskStatus::Open
            && t.assigned_user.is_none()
            && t.decider_role.as_deref() == Some("test_scoped_manager")));
        let in_marina = tasks
            .iter()
            .find(|t| t.scope_id == Some(1))
            .expect("the Marina task");
        assert_eq!(in_marina.scope_kind.as_deref(), Some("test_clinics"));
        let items = in_marina.items(app.db()).await.unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(ReviewTaskItem::is_pending));
        // The global group has NULL scope columns, as the table allows.
        let global = tasks
            .iter()
            .find(|t| t.scope_kind.is_none())
            .expect("global");
        assert_eq!(global.scope_id, None);
        assert_eq!(global.items(app.db()).await.unwrap().len(), 1);

        // A role held in Marina only is told about Marina's task, not the
        // other scopes'; a user with another role is told nothing.
        let bells = manager.unread_notifications(app.db()).await.unwrap();
        assert_eq!(bells.len(), 1);
        let message = bells[0].message().expect("a message");
        assert_eq!(message.title, "A party is on hold: 3 agreements to review");
        assert_eq!(bells[0].kind, REVIEW_NOTICE_KIND);
        assert!(
            outsider
                .unread_notifications(app.db())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[renox::test]
    async fn a_second_block_opens_no_second_task_while_one_is_open() {
        let (app, party, _views) = boot().await;
        let scope = Scope::new("test_clinics", 1);
        register_kind(
            "test_again",
            BlockEffect::Review,
            "test_again_manager",
            vec![agreement("again", 1, Some(scope.clone()))],
        );
        let manager = staff(&app, "Maha", "test_again_manager", &scope).await;
        let kinds = info("test_again");
        let first = open_review_tasks_for(app.state(), &kinds, PartyId(party.id), "on_hold")
            .await
            .unwrap();
        let second = open_review_tasks_for(app.state(), &kinds, PartyId(party.id), "on_hold")
            .await
            .unwrap();
        assert_eq!((first.len(), second.len()), (1, 0));
        assert_eq!(
            ReviewTask::open_for_party(app.db(), party.id)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            manager.unread_notifications(app.db()).await.unwrap().len(),
            1
        );
        // A closed task does not count: a later block opens a new one.
        let _outcome = first[0]
            .transition(crate::review_tasks::CLOSE)
            .by_system()
            .run(app.state())
            .await
            .unwrap();
        let third = open_review_tasks_for(app.state(), &kinds, PartyId(party.id), "on_hold")
            .await
            .unwrap();
        assert_eq!(third.len(), 1);
    }

    #[renox::test]
    async fn only_review_kinds_with_a_source_get_a_task() {
        let (app, party, _views) = boot().await;
        let refs = |kind: &str| vec![agreement(kind, 1, None)];
        register_kind("test_pausing", BlockEffect::Pause, "x", refs("pausing"));
        register_kind(
            "test_continuing",
            BlockEffect::Continue,
            "x",
            refs("continuing"),
        );
        // A review kind with no source can be listed but gets no task.
        AgreementKind::new("test_sourceless")
            .owner("test")
            .effect(BlockEffect::Review)
            .register();
        // A vertical turns a review kind into a continue one.
        register_kind(
            "test_overridden",
            BlockEffect::Review,
            "x",
            refs("overridden"),
        );
        BlockEffectOverride::kind("test_overridden")
            .effect(BlockEffect::Continue)
            .register();
        let kinds = infos(&[
            "test_pausing",
            "test_continuing",
            "test_sourceless",
            "test_overridden",
        ]);
        assert_eq!(kinds.len(), 4);
        let opened = open_review_tasks_for(app.state(), &kinds, PartyId(party.id), "on_hold")
            .await
            .unwrap();
        assert!(opened.is_empty());
        assert!(
            ReviewTask::open_for_party(app.db(), party.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[renox::test]
    async fn a_broken_source_does_not_stop_the_other_kinds() {
        let (app, party, _views) = boot().await;
        AgreementKind::new("test_broken")
            .owner("test")
            .effect(BlockEffect::Review)
            .source(Broken)
            .register();
        register_kind(
            "test_fine",
            BlockEffect::Review,
            "test_fine_manager",
            vec![agreement("fine", 1, None)],
        );
        let kinds = infos(&["test_broken", "test_fine"]);
        let result = open_review_tasks_for(app.state(), &kinds, PartyId(party.id), "on_hold").await;
        assert!(result.is_err());
        let tasks = ReviewTask::open_for_party(app.db(), party.id)
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].agreement_kind, "test_fine");
    }

    #[renox::test]
    async fn blocking_for_a_restricted_reason_stores_only_on_hold() {
        let (app, party, _views) = boot().await;
        register_kind(
            "test_event",
            BlockEffect::Review,
            "test_event_manager",
            vec![agreement("event", 1, None)],
        );
        // Through the listener: the block emits `PartyBlocked`.
        // The registry is process-wide, so another test's broken source may
        // make the listener report an error; the block itself has committed.
        let _blocked =
            crate::block_by_system(app.state(), &party, "sanctions", "confirmed match", None).await;
        let tasks = ReviewTask::open_for_party(app.db(), party.id)
            .await
            .unwrap();
        let task = tasks
            .iter()
            .find(|t| t.agreement_kind == "test_event")
            .expect("the listener opened a task");
        assert_eq!(task.display_reason, "on_hold");
        let lang = app.state().current_lang();
        assert_eq!(
            header(&lang, &BlockReasons::default(), &task.display_reason),
            "On hold"
        );
        let stored = format!("{task:?} {:?}", task.items(app.db()).await.unwrap());
        assert!(!stored.contains("sanctions"));
    }
}
