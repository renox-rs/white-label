//! Acceptance checks of #39 (blocking, freezing and reporting a confirmed
//! sanctions match), with a fixed clock, reviewers A and B, a partner, test
//! funds sources, a fake mailer and a raw SQL archive helper.
//!
//! The second-reviewer flow, the blocks and the records without a transition
//! (AC2, AC4, AC6) are in `confirm_match_apply.rs`, the task model (AC7, AC8
//! due times) in `compliance_tasks.rs`, restore alone in `restore.rs`, and
//! the pages in `compliance_pages.rs`. This file checks the whole path from
//! the flag to the freeze and report, in the order of the acceptance
//! criteria: AC1 (dates, event), AC3 (ended and merged links), AC5 (archive,
//! restore, unblock), AC7 (funds from sources), AC8 (due hours, overdue),
//! AC9 (partner and 403), AC10 (nothing to the party) and AC11 (no
//! `sanctions` outside the Block record).
//!
//! Test-only routes stand in where an `AuthUser` cannot be built.

mod support;

use std::sync::{Arc, Mutex, Once};

use renox::auth::permissions;
use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use renox::state_machine::{TransitionError, history};
use serde::Deserialize;
use wl_kernel::approvals::{self, ApprovalEnd};
use wl_kernel::held_funds::{FundsFuture, FundsSource, HeldFund, HeldFundsSource};
use wl_kernel::notify::NotifyChannel;
use wl_kernel::{
    ApprovalRequest, PartyId, PartyKind, PartyRestored, PartyStatus, ScreeningCompleted,
    ScreeningMatchConfirmed, ScreeningStatus,
};
use wl_notify::channels::ProviderFuture;
use wl_notify::{ChannelProvider, Message, Outgoing, SendError, WlNotify};
use wl_parties::compliance_tasks::ComplianceTask;
use wl_parties::confirm_match;
use wl_parties::screening_task::dubai_label;
use wl_parties::{
    BlockRecord, ComplianceTaskFund, ComplianceTaskKind, ComplianceTaskStatus, NewCandidate,
    NewParty, NewRelationship, Party, PartyStanding, RelationshipType, Screening, ScreeningRun,
    ScreeningTrigger, WlParties, create, permissions as perm, record_run, relate,
};

use support::{Booted, archive_party, boot_custom, set_dubai_clock};

const PDF: &[u8] = b"%PDF-1.7 sanctions screening result";

/// A fake e-mail provider that keeps what it was given.
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

/// Test funds sources: a deposit and prepaid credit for Omar, an open refund
/// for Falcon, nothing for anyone else.
struct Deposits;
struct Refunds;

async fn name_of(state: &AppState, party: PartyId) -> String {
    Party::find(&state.db, party.0)
        .await
        .ok()
        .flatten()
        .map(|p| p.legal_name_en)
        .unwrap_or_default()
}

impl FundsSource for Deposits {
    fn held_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> FundsFuture<'a, Vec<HeldFund>> {
        Box::pin(async move {
            Ok(if name_of(state, party).await == "Omar K." {
                vec![
                    HeldFund::new("Deposit D-1", "deposit", 150_000, "AED"),
                    HeldFund::new("Credit C-3", "prepaid_credit", 20_000, "AED"),
                ]
            } else {
                Vec::new()
            })
        })
    }
}

impl FundsSource for Refunds {
    fn held_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> FundsFuture<'a, Vec<HeldFund>> {
        Box::pin(async move {
            Ok(if name_of(state, party).await == "Falcon Holdings LLC" {
                vec![HeldFund::new("Refund R-7", "refund", 25_050, "AED")]
            } else {
                Vec::new()
            })
        })
    }
}

static SOURCES: Once = Once::new();

fn register_sources() {
    SOURCES.call_once(|| {
        HeldFundsSource::new("accept-deposits")
            .owner("wl-parties-tests")
            .source(Deposits)
            .register();
        HeldFundsSource::new("accept-refunds")
            .owner("wl-parties-tests")
            .source(Refunds)
            .register();
    });
}

#[derive(Deserialize)]
struct NoteForm {
    note: String,
}

/// The signed-in user confirms the match: `done`, `pending` or a status.
async fn confirm_route(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<NoteForm>,
) -> Result<Response> {
    let screening = Screening::find_or_404(&state.db, id).await?;
    match confirm_match::confirm_match(&state, &user, &screening, &form.note).await {
        Ok(outcome) if outcome.is_pending() => Ok("pending".into_response()),
        Ok(_) => Ok("done".into_response()),
        Err(TransitionError::Forbidden) => Ok(StatusCode::FORBIDDEN.into_response()),
        Err(TransitionError::Guard(_)) => Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response()),
        Err(TransitionError::NotFrom(_)) => Ok(StatusCode::CONFLICT.into_response()),
        Err(other) => Err(Error::permanent_message(other)),
    }
}

/// The signed-in user restores a party.
async fn restore_route(
    State(state): State<AppState>,
    user: AuthUser,
    Path(party_id): Path<i64>,
) -> Result<Response> {
    match wl_parties::restore(&state, &user, party_id).await {
        Ok(()) => Ok("restored".into_response()),
        Err(TransitionError::Forbidden) => Ok(StatusCode::FORBIDDEN.into_response()),
        Err(TransitionError::NotFrom(_)) => Ok(StatusCode::CONFLICT.into_response()),
        Err(other) => Err(Error::permanent_message(other)),
    }
}

struct TestRoutes;

impl Module for TestRoutes {
    fn name(&self) -> &'static str {
        "accept-routes"
    }

    fn routes(&self) -> Routes {
        Routes::new()
            .post("/t/confirm/{id}", confirm_route)
            .post("/t/restore/{party_id}", restore_route)
    }
}

struct World {
    t: Booted,
    mailer: Mailer,
    confirmed: Arc<Mutex<Vec<ScreeningMatchConfirmed>>>,
    completed: Arc<Mutex<Vec<ScreeningCompleted>>>,
    restored: Arc<Mutex<Vec<PartyRestored>>>,
    /// Reviewer A: runs `confirm_match`.
    a: User,
    /// Reviewer B: the second reviewer.
    b: User,
    /// Reads the tasks but may not review.
    reader: User,
    /// Sees parties only.
    partner: User,
    omar: Party,
}

async fn compliance(t: &Booted, name: &str) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    permissions::define_role(
        t.db(),
        "compliance",
        &[
            perm::VIEW,
            perm::BLOCK,
            perm::BLOCK_REASON_VIEW,
            perm::SCREENING_REVIEW,
        ],
    )
    .await
    .unwrap();
    user.assign_role(t.db(), "compliance").await.unwrap();
    user
}

async fn party(t: &Booted, kind: PartyKind, name: &str) -> Party {
    create(
        t.app.state(),
        NewParty {
            kind,
            legal_name_en: name.into(),
            source: "test".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

async fn world_with(module: WlParties) -> World {
    register_sources();
    let mailer = Mailer::default();
    let provider = mailer.clone();
    let confirmed = Arc::new(Mutex::new(Vec::new()));
    let completed = Arc::new(Mutex::new(Vec::new()));
    let restored = Arc::new(Mutex::new(Vec::new()));
    let (c, d, r) = (
        Arc::clone(&confirmed),
        Arc::clone(&completed),
        Arc::clone(&restored),
    );
    let t = boot_custom(module.screening(true), false, move |app| {
        app.module(TestRoutes)
            .module(WlNotify::new().provider(provider))
            .listen(move |event: ScreeningMatchConfirmed, _state: AppState| {
                let c = Arc::clone(&c);
                async move {
                    c.lock().unwrap().push(event);
                    Ok(())
                }
            })
            .listen(move |event: ScreeningCompleted, _state: AppState| {
                let d = Arc::clone(&d);
                async move {
                    d.lock().unwrap().push(event);
                    Ok(())
                }
            })
            .listen(move |event: PartyRestored, _state: AppState| {
                let r = Arc::clone(&r);
                async move {
                    r.lock().unwrap().push(event);
                    Ok(())
                }
            })
    })
    .await;
    t.app
        .kernel()
        .call("notify:install", Vec::<String>::new())
        .await
        .expect("notify:install runs");
    let a = compliance(&t, "Alia").await;
    let b = compliance(&t, "Basma").await;
    let reader = t.user("Rana", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    let partner = t.user("Pia", &[perm::VIEW]).await;
    let omar = party(&t, PartyKind::Person, "Omar K.").await;
    World {
        t,
        mailer,
        confirmed,
        completed,
        restored,
        a,
        b,
        reader,
        partner,
        omar,
    }
}

async fn world() -> World {
    world_with(WlParties::new()).await
}

impl World {
    /// A possible match for `who`, flagged at the current (fixed) clock.
    async fn flag_for(&self, who: &Party) -> Screening {
        let candidate = NewCandidate {
            name: who.legal_name_en.clone(),
            list_name: "UN consolidated".into(),
            score: 0.91,
            list_entry: Some("QDi.001".into()),
        };
        let screening = self
            .t
            .app
            .at_travelled_time(record_run(
                self.t.app.state(),
                who,
                ScreeningRun::automatic(ScreeningTrigger::Review, vec![candidate]),
                None,
            ))
            .await
            .unwrap();
        self.t.app.run_all_jobs().await;
        screening
    }

    async fn flag(&self) -> Screening {
        self.flag_for(&self.omar).await
    }

    async fn confirm(&self, user: &User, screening: &Screening, note: &str) -> String {
        self.t.app.acting_as(user);
        let response = self
            .t
            .app
            .post(&format!("/t/confirm/{}", screening.id), &[("note", note)])
            .await;
        if response.status.is_success() {
            response.text()
        } else {
            response.status.as_u16().to_string()
        }
    }

    /// A confirms, B approves; the jobs are worked off.
    async fn confirm_and_approve(&self, screening: &Screening) {
        assert_eq!(
            self.confirm(&self.a, screening, "Same DOB").await,
            "pending"
        );
        let request = ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(confirm_match::SUBJECT_TYPE))
            .where_(ApprovalRequest::SUBJECT_ID.eq(screening.id))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("a request");
        let decided = self
            .t
            .app
            .at_travelled_time(approvals::approve(
                self.t.app.state(),
                request.id,
                &self.b,
                "ok",
            ))
            .await
            .unwrap()
            .expect("the request ended");
        assert_eq!(decided.decision, ApprovalEnd::Approved);
        self.t.app.run_all_jobs().await;
    }

    async fn status(&self, screening: &Screening) -> ScreeningStatus {
        Screening::find(self.t.db(), screening.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }

    async fn tasks(&self, screening: &Screening) -> Vec<ComplianceTask> {
        ComplianceTask::for_screening(self.t.db(), screening.id)
            .await
            .unwrap()
    }

    async fn task(&self, screening: &Screening, kind: ComplianceTaskKind) -> ComplianceTask {
        self.tasks(screening)
            .await
            .into_iter()
            .find(|t| t.kind == kind)
            .expect("a task")
    }

    async fn relate(&self, company: &Party, kind: RelationshipType) {
        self.relate_between(company, kind, None, None).await;
    }

    async fn relate_between(
        &self,
        company: &Party,
        kind: RelationshipType,
        from: Option<NaiveDate>,
        until: Option<NaiveDate>,
    ) {
        relate(
            self.t.app.state(),
            NewRelationship {
                from: self.omar.id,
                to: company.id,
                relation_type: kind,
                valid_from: from,
                valid_until: until,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    async fn messages(&self) -> Vec<Message> {
        Message::query()
            .order_by(Message::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    /// The titles of `user`'s unread bell entries.
    async fn bells(&self, user: &User) -> Vec<(String, String)> {
        user.unread_notifications(self.t.db())
            .await
            .unwrap()
            .iter()
            .filter_map(|b| b.message())
            .map(|m| (m.title.clone(), m.body.clone().unwrap_or_default()))
            .collect()
    }
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// AC1: flagged 2026-11-03 14:00 Dubai, confirmed 2026-11-04 09:00; both
/// tasks are due 2026-11-04 14:00; the event reaches a compliance listener
/// once and the confirm emits no `ScreeningCompleted`.
#[renox::test]
async fn a_confirmed_match_blocks_the_person_and_opens_two_tasks_due_24_hours_after_the_flag() {
    let w = world().await;
    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;

    assert_eq!(w.status(&screening).await, ScreeningStatus::ConfirmedMatch);
    let omar = w.t.reload(&w.omar).await;
    assert_eq!(omar.status, PartyStatus::Blocked);
    assert_eq!(omar.hold_reason.as_deref(), Some("on_hold"));
    let records = BlockRecord::open_for(w.t.db(), omar.id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason_code, "sanctions");
    assert!(records[0].restricted);
    assert_eq!(records[0].blocked_by, Some(w.a.id));

    let tasks = w.tasks(&screening).await;
    let kinds: Vec<_> = tasks.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        [ComplianceTaskKind::Freeze, ComplianceTaskKind::Report]
    );
    for task in &tasks {
        assert_eq!(dubai_label(task.due_at), "2026-11-04 14:00");
        assert_eq!(task.status, ComplianceTaskStatus::Open);
    }
    assert_eq!(w.confirmed.lock().unwrap().len(), 1);
    assert!(w.completed.lock().unwrap().is_empty());
}

/// AC3: UBO and director companies are blocked; a merged company, an ended
/// relationship and a shareholder link are not.
#[renox::test]
async fn only_current_ubo_director_and_signatory_companies_are_blocked() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    let dune = party(&w.t, PartyKind::Company, "Dune Logistics").await;
    let sign = party(&w.t, PartyKind::Company, "Signatory Trading").await;
    let old = party(&w.t, PartyKind::Company, "Old Holdings").await;
    let shares = party(&w.t, PartyKind::Company, "Shares LLC").await;
    let merged = party(&w.t, PartyKind::Company, "Merged LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    w.relate(&dune, RelationshipType::DirectorOf).await;
    w.relate(&sign, RelationshipType::SignatoryOf).await;
    w.relate_between(
        &old,
        RelationshipType::UboOf,
        Some(day(2019, 1, 1)),
        Some(day(2020, 1, 1)),
    )
    .await;
    w.relate(&shares, RelationshipType::ShareholderOf).await;
    w.relate(&merged, RelationshipType::UboOf).await;
    renox::db::sql("UPDATE wl_parties_parties SET status = 'merged' WHERE id = ?")
        .bind(merged.id)
        .execute(w.t.db())
        .await
        .unwrap();

    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;

    for blocked in [&w.omar, &falcon, &dune, &sign] {
        let after = w.t.reload(blocked).await;
        assert_eq!(
            after.status,
            PartyStatus::Blocked,
            "{}",
            after.legal_name_en
        );
        assert_eq!(after.hold_reason.as_deref(), Some("on_hold"));
        let records = BlockRecord::open_for(w.t.db(), after.id).await.unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].restricted && records[0].reason_code == "sanctions");
    }
    for untouched in [&old, &shares] {
        assert_eq!(w.t.reload(untouched).await.status, PartyStatus::Active);
    }
    assert_eq!(w.t.reload(&merged).await.status, PartyStatus::Merged);
    for kind in [ComplianceTaskKind::Freeze, ComplianceTaskKind::Report] {
        let ids: Vec<i64> = w
            .task(&screening, kind)
            .await
            .parties(w.t.db())
            .await
            .unwrap()
            .iter()
            .map(|p| p.party_id)
            .collect();
        assert_eq!(ids, [w.omar.id, falcon.id, dune.id, sign.id]);
    }
}

/// AC5: an archived related company stays archived with an open record named
/// in the freeze task; `restore` makes it `blocked`, emitting `PartyRestored`
/// and a neutral `PartyBlocked`; only `unblock` with evidence, by someone who
/// did not run the match, makes it `active`. A party without a record
/// restores to `active`.
#[renox::test]
async fn an_archived_company_is_restored_blocked_and_only_unblock_lifts_it() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    let plain = party(&w.t, PartyKind::Company, "Plain Archived LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    archive_party(&w.t, &falcon).await;
    archive_party(&w.t, &plain).await;

    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;

    let after = w.t.reload(&falcon).await;
    assert_eq!(after.status, PartyStatus::Archived);
    let records = BlockRecord::open_for(w.t.db(), falcon.id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert!(records[0].restricted);
    let freeze = w.task(&screening, ComplianceTaskKind::Freeze).await;
    let named = freeze.parties(w.t.db()).await.unwrap();
    let entry = named.iter().find(|p| p.party_id == falcon.id).unwrap();
    assert_eq!(entry.standing, PartyStanding::Archived);
    assert_eq!(entry.block_record_id, Some(records[0].id));
    assert_eq!(w.t.blocked().len(), 1, "only the person so far");

    // Restore: blocked, not active.
    let restorer = w.t.user("Rita", &[perm::VIEW, perm::ARCHIVE]).await;
    w.t.app.acting_as(&restorer);
    w.t.app
        .post(&format!("/t/restore/{}", falcon.id), &[("x", "1")])
        .await
        .assert_ok();
    let restored = w.t.reload(&falcon).await;
    assert_eq!(restored.status, PartyStatus::Blocked);
    assert_eq!(restored.hold_reason.as_deref(), Some("on_hold"));
    assert_eq!(w.restored.lock().unwrap().len(), 1);
    let blocked = w.t.blocked();
    assert_eq!(blocked.len(), 2);
    assert_eq!(blocked[1].display_reason, "on_hold");
    assert_eq!(blocked[1].party, PartyId(falcon.id));

    // A party without an open record comes back active.
    w.t.app
        .post(&format!("/t/restore/{}", plain.id), &[("x", "1")])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&plain).await.status, PartyStatus::Active);

    // The reviewer who ran the match cannot lift it; B can, with evidence.
    let unblock = |user: &User| {
        w.t.app.acting_as(user);
        let fields = [("note", "false match"), ("basis", "false_match")];
        let files = [("files", "m.pdf", PDF)];
        let url = format!("/parties/{}/unblock", falcon.id);
        let app = &w.t.app;
        async move { app.htmx().post_multipart(&url, &fields, &files).await }
    };
    unblock(&w.a).await.assert_status(422);
    assert_eq!(w.t.reload(&falcon).await.status, PartyStatus::Blocked);
    unblock(&w.b).await.assert_ok();
    assert_eq!(w.t.reload(&falcon).await.status, PartyStatus::Active);
    assert_eq!(w.t.unblocked().len(), 1);
    assert_eq!(w.t.unblocked()[0].display_reason, "on_hold");
    assert_eq!(
        w.t.reload(&w.omar).await.status,
        PartyStatus::Blocked,
        "the person's own record is still open"
    );
}

/// AC7: the freeze task is pre-filled from the registered sources (deposit,
/// prepaid credit, open refund) for each party named; the compliance role
/// confirms and edits lines, adds one by hand, records the freeze; the
/// report stores its reference and date.
#[renox::test]
async fn the_freeze_task_is_filled_from_the_sources_and_both_tasks_complete() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;

    let freeze = w.task(&screening, ComplianceTaskKind::Freeze).await;
    let report = w.task(&screening, ComplianceTaskKind::Report).await;
    let funds = freeze.funds(w.t.db()).await.unwrap();
    let lines: Vec<_> = funds
        .iter()
        .map(|f| (f.label.as_str(), f.kind.as_str(), f.amount_fils, f.party_id))
        .collect();
    assert_eq!(
        lines,
        [
            ("Deposit D-1", "deposit", 150_000, Some(w.omar.id)),
            ("Credit C-3", "prepaid_credit", 20_000, Some(w.omar.id)),
            ("Refund R-7", "refund", 25_050, Some(falcon.id)),
        ]
    );
    assert!(funds.iter().all(|f| !f.confirmed));
    assert!(report.funds(w.t.db()).await.unwrap().is_empty());

    let freeze_url = format!("/parties/compliance-tasks/{}", freeze.id);
    w.t.app.acting_as(&w.a);
    w.t.app
        .get(&freeze_url)
        .await
        .assert_ok()
        .assert_see("Omar K.")
        .assert_see("Falcon Holdings LLC")
        .assert_see("Deposit D-1")
        .assert_see("Refund R-7");

    // Confirm the first line, edit the credit, add one by hand.
    w.t.app
        .post(
            &format!("{freeze_url}/funds/{}", funds[0].id),
            &[("amount", "1,500.00"), ("state", "confirmed")],
        )
        .await
        .assert_ok();
    w.t.app
        .post(
            &format!("{freeze_url}/funds/{}", funds[1].id),
            &[("amount", "150")],
        )
        .await
        .assert_ok();
    w.t.app
        .post(
            &format!("{freeze_url}/funds"),
            &[
                ("label", "Cash held at site"),
                ("kind", "other"),
                ("amount", "10"),
            ],
        )
        .await
        .assert_ok();
    let funds = freeze.funds(w.t.db()).await.unwrap();
    assert_eq!(funds.len(), 4);
    let by_label =
        |label: &str| -> &ComplianceTaskFund { funds.iter().find(|f| f.label == label).unwrap() };
    assert!(by_label("Deposit D-1").confirmed);
    assert_eq!(by_label("Credit C-3").amount_fils, 15_000);
    assert!(!by_label("Credit C-3").confirmed);
    assert_eq!(by_label("Cash held at site").amount_fils, 1_000);

    // The reader (403) and the partner (404) cannot record the freeze.
    w.t.app.acting_as(&w.reader);
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_forbidden();
    w.t.app.acting_as(&w.partner);
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_not_found();
    let still = ComplianceTask::find(w.t.db(), freeze.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.status, ComplianceTaskStatus::Open);

    w.t.app.acting_as(&w.a);
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_ok();
    let done = ComplianceTask::find(w.t.db(), freeze.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, ComplianceTaskStatus::Done);
    assert!(done.frozen_at.is_some());
    assert_eq!(done.completed_by, Some(w.a.id));

    let report_url = format!("/parties/compliance-tasks/{}", report.id);
    w.t.app
        .post(
            &format!("{report_url}/complete"),
            &[
                ("reference", "GOAML-2026-17"),
                ("reported_on", "2026-11-04"),
            ],
        )
        .await
        .assert_ok();
    let report = ComplianceTask::find(w.t.db(), report.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(report.status, ComplianceTaskStatus::Done);
    assert_eq!(report.reference.as_deref(), Some("GOAML-2026-17"));
    assert!(report.reported_on.is_some());
}

/// AC8: with `report_task_due_hours(48)` the report is due 48 hours after the
/// flag and the freeze 24; a confirmation after the 24 hours creates a task
/// already overdue; tasks that exist are not changed by a later one.
#[renox::test]
async fn the_due_hours_follow_the_settings_and_a_late_confirmation_is_overdue() {
    let w = world_with(WlParties::new().report_task_due_hours(48)).await;
    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    // Confirmed 2026-11-05 09:00, 43 hours after the flag.
    let confirmed_at = set_dubai_clock(&w.t, 5, 9);
    w.confirm_and_approve(&screening).await;

    let freeze = w.task(&screening, ComplianceTaskKind::Freeze).await;
    let report = w.task(&screening, ComplianceTaskKind::Report).await;
    assert_eq!(dubai_label(freeze.due_at), "2026-11-04 14:00");
    assert_eq!(dubai_label(report.due_at), "2026-11-05 14:00");
    assert_eq!(report.due_at - freeze.due_at, Duration::hours(24));
    assert!(freeze.is_overdue(confirmed_at), "no clamp to the future");
    assert!(!report.is_overdue(confirmed_at));

    // A second person, flagged and confirmed later: the first tasks stay.
    let layla = party(&w.t, PartyKind::Person, "Layla M.").await;
    set_dubai_clock(&w.t, 5, 10);
    let second = w.flag_for(&layla).await;
    w.confirm_and_approve(&second).await;
    let again = w.tasks(&screening).await;
    assert_eq!(again[0].due_at, freeze.due_at);
    assert_eq!(again[1].due_at, report.due_at);
    let later = w.task(&second, ComplianceTaskKind::Report).await;
    assert_eq!(dubai_label(later.due_at), "2026-11-07 10:00");
}

/// AC9: a partner with `parties.view` only sees "On hold" for the person and
/// the company, with no code, task, count or nav link, and the task routes
/// answer 404; a holder sees the tasks, in the nav too.
#[renox::test]
async fn a_partner_sees_on_hold_and_no_task() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;
    let freeze = w.task(&screening, ComplianceTaskKind::Freeze).await;
    let freeze_url = format!("/parties/compliance-tasks/{}", freeze.id);
    let tasks_url = "/parties/compliance-tasks";

    w.t.app.acting_as(&w.partner);
    for party in [&w.omar, &falcon] {
        let page = w.t.app.get(&format!("/parties/{}", party.id)).await;
        page.assert_ok()
            .assert_see("On hold")
            .assert_dont_see("anction")
            .assert_dont_see(tasks_url);
    }
    w.t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_see("On hold")
        .assert_dont_see(tasks_url)
        .assert_dont_see("anction");
    w.t.app.get(tasks_url).await.assert_not_found();
    w.t.app.get(&freeze_url).await.assert_not_found();
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_not_found();
    assert!(w.bells(&w.partner).await.is_empty());

    // The holder who may not review reads the tasks and cannot complete.
    w.t.app.acting_as(&w.reader);
    w.t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_see(tasks_url);
    w.t.app.get(tasks_url).await.assert_ok();
    w.t.app.get(&freeze_url).await.assert_ok();
    w.t.app
        .post(&format!("{freeze_url}/complete"), &[("frozen_at", "")])
        .await
        .assert_forbidden();
}

/// AC10: nothing reaches the party: the confirm sends no e-mail and queues
/// no message for the person or the companies; each holder gets one bell
/// entry with no name, list or reason code.
#[renox::test]
async fn nothing_reaches_the_party_and_the_bell_names_nobody() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    // The flag alert went to the compliance holders already.
    let mails_before = w.mailer.0.lock().unwrap().len();
    let messages_before = w.messages().await.len();
    assert!(mails_before > 0);

    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;

    assert_eq!(w.mailer.0.lock().unwrap().len(), mails_before);
    let messages = w.messages().await;
    assert_eq!(messages.len(), messages_before, "nothing queued at confirm");
    for m in &messages {
        assert!(m.party_id != Some(w.omar.id) && m.party_id != Some(falcon.id));
        assert!(
            m.contact_of_party_id != Some(w.omar.id) && m.contact_of_party_id != Some(falcon.id)
        );
    }
    for holder in [&w.a, &w.b] {
        let ours: Vec<_> = w
            .bells(holder)
            .await
            .into_iter()
            .filter(|(title, _)| title == "Freeze and report tasks opened")
            .collect();
        assert_eq!(ours.len(), 1, "{}", holder.name);
        let text = format!("{} {}", ours[0].0, ours[0].1).to_lowercase();
        for hidden in ["omar", "falcon", "un consolidated", "sanctions", "on_hold"] {
            assert!(!text.contains(hidden), "{hidden} in {text}");
        }
    }
    // The partner and the reader (not a reviewer) get no such entry.
    assert!(w.bells(&w.partner).await.is_empty());
}

/// AC11: `sanctions` is in no event payload, no state transition row, and no
/// page or CSV a non-holder can reach.
#[renox::test]
async fn the_sanctions_code_stays_in_the_block_record() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    set_dubai_clock(&w.t, 3, 14);
    let screening = w.flag().await;
    set_dubai_clock(&w.t, 4, 9);
    w.confirm_and_approve(&screening).await;
    archive_party(&w.t, &w.omar).await; // a raw SQL change leaves no row
    let restorer = w.t.user("Rita", &[perm::VIEW, perm::ARCHIVE]).await;
    w.t.app.acting_as(&restorer);
    w.t.app
        .post(&format!("/t/restore/{}", w.omar.id), &[("x", "1")])
        .await
        .assert_ok();

    // Events.
    let events = format!(
        "{:?} {:?} {:?} {:?}",
        w.t.blocked(),
        w.confirmed.lock().unwrap(),
        w.restored.lock().unwrap(),
        w.completed.lock().unwrap()
    )
    .to_lowercase();
    assert!(!events.contains("sanctions"), "{events}");
    assert!(events.contains("on_hold"));

    // State transition rows of the parties and the screening.
    for (table, id) in [
        (Party::TABLE, w.omar.id),
        (Party::TABLE, falcon.id),
        (Screening::TABLE, screening.id),
    ] {
        let rows = history(w.t.db(), table, id).await.unwrap();
        assert!(!rows.is_empty(), "{table} {id}");
        let text = format!("{rows:?}").to_lowercase();
        assert!(!text.contains("sanctions"), "{table} {id}: {text}");
    }

    // Pages and the CSV of a non-holder who may export.
    let exporter = w.t.user("Pax", &[perm::VIEW, perm::EXPORT]).await;
    w.t.app.acting_as(&exporter);
    let csv = w.t.app.get("/parties?export=csv").await.assert_ok().text();
    assert!(!csv.to_lowercase().contains("sanctions"), "{csv}");
    assert!(csv.contains("On hold"));
    for url in [
        "/parties".to_owned(),
        format!("/parties/{}", w.omar.id),
        format!("/parties/{}", falcon.id),
    ] {
        let text = w.t.app.get(&url).await.assert_ok().text();
        assert!(!text.to_lowercase().contains("sanctions"), "{url}");
    }
}
