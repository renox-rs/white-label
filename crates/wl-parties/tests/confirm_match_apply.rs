//! `confirm_match` and what it does (#39, part 4): the second reviewer, the
//! blocks, the records without a transition, the tasks, the alert and the
//! event, and a rerun. The full acceptance checks are in part 6.
//!
//! A test-only route confirms as the signed-in user (`acting_as`).

mod support;

use std::sync::{Arc, Mutex};

use renox::auth::permissions;
use renox::prelude::*;
use renox::state_machine::{TransitionError, history};
use serde::Deserialize;
use wl_kernel::approvals::{self, ApprovalEnd};
use wl_kernel::{
    ApprovalRequest, PartyKind, PartyStatus, ScreeningCompleted, ScreeningMatchConfirmed,
    ScreeningStatus,
};
use wl_parties::compliance_tasks::ComplianceTask;
use wl_parties::confirm_match::{self, apply_confirmed_match};
use wl_parties::{
    BlockRecord, ComplianceTaskKind, NewCandidate, NewParty, NewRelationship, Party, PartyStanding,
    RelationshipType, Screening, ScreeningRun, ScreeningTask, ScreeningTaskStatus,
    ScreeningTrigger, WlParties, create, permissions as perm, record_run, relate,
};

use support::{Booted, boot_custom};

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
        // A request for it is already pending.
        Err(TransitionError::Failed(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))) => {
            Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response())
        }
        Err(other) => Err(Error::permanent_message(other)),
    }
}

struct ConfirmRoute;

impl Module for ConfirmRoute {
    fn name(&self) -> &'static str {
        "confirm-route"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/t/confirm/{id}", confirm_route)
    }
}

struct World {
    t: Booted,
    confirmed: Arc<Mutex<Vec<ScreeningMatchConfirmed>>>,
    completed: Arc<Mutex<Vec<ScreeningCompleted>>>,
    /// Reviewer A: runs `confirm_match`.
    a: User,
    /// Reviewer B: the second reviewer.
    b: User,
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
    let confirmed = Arc::new(Mutex::new(Vec::new()));
    let completed = Arc::new(Mutex::new(Vec::new()));
    let (c, d) = (Arc::clone(&confirmed), Arc::clone(&completed));
    let t = boot_custom(module.screening(true), false, move |app| {
        app.module(ConfirmRoute)
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
    })
    .await;
    let a = compliance(&t, "Alia").await;
    let b = compliance(&t, "Basma").await;
    let omar = party(&t, PartyKind::Person, "Omar K.").await;
    World {
        t,
        confirmed,
        completed,
        a,
        b,
        omar,
    }
}

async fn world() -> World {
    world_with(WlParties::new()).await
}

impl World {
    /// A possible match for `omar`.
    async fn flag(&self) -> Screening {
        let candidate = NewCandidate {
            name: "Omar K.".into(),
            list_name: "UN consolidated".into(),
            score: 0.91,
            list_entry: Some("QDi.001".into()),
        };
        record_run(
            self.t.app.state(),
            &self.omar,
            ScreeningRun::automatic(ScreeningTrigger::Review, vec![candidate]),
            None,
        )
        .await
        .unwrap()
    }

    /// `user` confirms through the route; the response body or status.
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

    /// Confirms as A and approves as B.
    async fn confirm_and_approve(&self, screening: &Screening) {
        assert_eq!(
            self.confirm(&self.a, screening, "Same DOB").await,
            "pending"
        );
        let request = self.requests(screening).await.remove(0);
        let decided = approvals::approve(self.t.app.state(), request.id, &self.b, "ok")
            .await
            .unwrap()
            .expect("the request ended");
        assert_eq!(decided.decision, ApprovalEnd::Approved);
    }

    async fn requests(&self, screening: &Screening) -> Vec<ApprovalRequest> {
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_TYPE.eq(confirm_match::SUBJECT_TYPE))
            .where_(ApprovalRequest::SUBJECT_ID.eq(screening.id))
            .get(self.t.db())
            .await
            .unwrap()
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

    async fn history_of(&self, party: &Party) -> Vec<String> {
        history(self.t.db(), Party::TABLE, party.id)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.transition)
            .collect()
    }

    async fn set_status(&self, party: &Party, status: &str) {
        renox::db::sql("UPDATE wl_parties_parties SET status = ? WHERE id = ?")
            .bind(status)
            .bind(party.id)
            .execute(self.t.db())
            .await
            .unwrap();
    }

    async fn relate(&self, company: &Party, kind: RelationshipType) {
        relate(
            self.t.app.state(),
            NewRelationship {
                from: self.omar.id,
                to: company.id,
                relation_type: kind,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
}

#[renox::test]
async fn a_second_reviewer_confirms_and_the_person_is_blocked_with_tasks_and_an_alert() {
    let w = world().await;
    let screening = w.flag().await;

    // No note: refused. A note: waits for the second reviewer.
    assert_eq!(w.confirm(&w.a, &screening, "  ").await, "422");
    assert_eq!(w.confirm(&w.a, &screening, "Same DOB").await, "pending");
    assert_eq!(w.status(&screening).await, ScreeningStatus::PossibleMatch);
    assert_eq!(w.t.reload(&w.omar).await.status, PartyStatus::Active);
    assert!(w.tasks(&screening).await.is_empty());
    // A second request while one waits is refused.
    assert_eq!(w.confirm(&w.a, &screening, "Again").await, "422");

    // A cannot approve their own request.
    let request = w.requests(&screening).await.remove(0);
    assert!(matches!(
        approvals::approve(w.t.app.state(), request.id, &w.a, "").await,
        Err(Error::Forbidden)
    ));
    assert_eq!(w.status(&screening).await, ScreeningStatus::PossibleMatch);

    let decided = approvals::approve(w.t.app.state(), request.id, &w.b, "ok")
        .await
        .unwrap()
        .expect("the request ended");
    assert_eq!(decided.decision, ApprovalEnd::Approved);

    let stored = Screening::find(w.t.db(), screening.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ScreeningStatus::ConfirmedMatch);
    assert_eq!(stored.review_note.as_deref(), Some("Same DOB"));
    let omar = w.t.reload(&w.omar).await;
    assert_eq!(omar.status, PartyStatus::Blocked);
    assert_eq!(omar.hold_reason.as_deref(), Some("on_hold"));
    let records = BlockRecord::open_for(w.t.db(), omar.id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason_code, "sanctions");
    assert!(records[0].restricted);
    assert_eq!(
        records[0].blocked_by,
        Some(w.a.id),
        "the reviewer who ran it"
    );
    assert_eq!(records[0].screening_id, Some(screening.id));
    assert_eq!(w.history_of(&omar).await, ["block_by_system"]);
    // No `sanctions` in the event or the history reason.
    assert_eq!(w.t.blocked().len(), 1);
    assert_eq!(w.t.blocked()[0].display_reason, "on_hold");
    let rows = history(w.t.db(), Party::TABLE, omar.id).await.unwrap();
    assert_eq!(rows[0].reason.as_deref(), Some("on_hold"));

    // One freeze and one report task, due 24 hours after the flag.
    let review = ScreeningTask::find(w.t.db(), stored.task_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(review.status, ScreeningTaskStatus::Closed);
    let tasks = w.tasks(&screening).await;
    let kinds: Vec<_> = tasks.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        [ComplianceTaskKind::Freeze, ComplianceTaskKind::Report]
    );
    for task in &tasks {
        assert_eq!(
            task.due_at,
            review.flagged_at + renox::chrono::Duration::hours(24)
        );
        let named = task.parties(w.t.db()).await.unwrap();
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].party_id, omar.id);
        assert_eq!(named[0].standing, PartyStanding::Blocked);
        assert_eq!(named[0].block_record_id, Some(records[0].id));
    }

    // The event once, to compliance listeners; no `ScreeningCompleted` for
    // the confirm (the one of the flag run is not a completed one either).
    assert_eq!(w.confirmed.lock().unwrap().len(), 1);
    assert!(w.completed.lock().unwrap().is_empty());

    // Each holder has one entry without a name, a list or a reason.
    for holder in [&w.a, &w.b] {
        let bells = holder.unread_notifications(w.t.db()).await.unwrap();
        let ours: Vec<_> = bells
            .iter()
            .filter_map(|b| b.message())
            .filter(|m| m.title == "Freeze and report tasks opened")
            .collect();
        assert_eq!(ours.len(), 1, "{}", holder.name);
        let text = format!(
            "{} {}",
            ours[0].title,
            ours[0].body.clone().unwrap_or_default()
        );
        for secret in ["Omar", "UN consolidated", "sanctions"] {
            assert!(!text.contains(secret), "{text}");
        }
    }
}

#[renox::test]
async fn a_rejection_leaves_the_screening_and_the_party_alone() {
    let w = world().await;
    let screening = w.flag().await;
    assert_eq!(w.confirm(&w.a, &screening, "Same DOB").await, "pending");
    let request = w.requests(&screening).await.remove(0);
    approvals::reject(w.t.app.state(), request.id, &w.b, "Different person")
        .await
        .unwrap();
    assert_eq!(w.status(&screening).await, ScreeningStatus::PossibleMatch);
    assert_eq!(w.t.reload(&w.omar).await.status, PartyStatus::Active);
    assert!(w.tasks(&screening).await.is_empty());
    assert!(w.confirmed.lock().unwrap().is_empty());
}

#[renox::test]
async fn related_companies_are_blocked_too_and_the_rest_are_not() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    let dune = party(&w.t, PartyKind::Company, "Dune Logistics").await;
    let shares = party(&w.t, PartyKind::Company, "Shares LLC").await;
    let merged = party(&w.t, PartyKind::Company, "Merged LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    w.relate(&dune, RelationshipType::DirectorOf).await;
    w.relate(&shares, RelationshipType::ShareholderOf).await;
    w.relate(&merged, RelationshipType::UboOf).await;
    w.set_status(&merged, "merged").await;

    let screening = w.flag().await;
    w.confirm_and_approve(&screening).await;

    for company in [&falcon, &dune] {
        let after = w.t.reload(company).await;
        assert_eq!(
            after.status,
            PartyStatus::Blocked,
            "{}",
            company.legal_name_en
        );
        assert_eq!(after.hold_reason.as_deref(), Some("on_hold"));
        let records = BlockRecord::open_for(w.t.db(), company.id).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].reason_code, "sanctions");
    }
    assert_eq!(w.t.reload(&shares).await.status, PartyStatus::Active);
    assert_eq!(w.t.reload(&merged).await.status, PartyStatus::Merged);

    let tasks = w.tasks(&screening).await;
    assert_eq!(tasks.len(), 2, "one of each, however many parties");
    for task in &tasks {
        let ids: Vec<i64> = task
            .parties(w.t.db())
            .await
            .unwrap()
            .iter()
            .map(|p| p.party_id)
            .collect();
        assert_eq!(ids, [w.omar.id, falcon.id, dune.id]);
    }
}

#[renox::test]
async fn with_the_setting_off_only_the_person_is_blocked() {
    let w = world_with(WlParties::new().related_company_block(false)).await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    let screening = w.flag().await;
    w.confirm_and_approve(&screening).await;
    assert_eq!(w.t.reload(&w.omar).await.status, PartyStatus::Blocked);
    assert_eq!(w.t.reload(&falcon).await.status, PartyStatus::Active);
    let freeze = w.tasks(&screening).await.remove(0);
    assert_eq!(freeze.parties(w.t.db()).await.unwrap().len(), 1);
}

#[renox::test]
async fn a_blocked_or_archived_party_only_gets_a_restricted_record() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    // Omar is already blocked for another reason.
    let mlro = w.a.clone();
    support::block_party(&w.t, &mlro, &w.omar, "non_payment", "3 bounced cheques").await;
    assert_eq!(w.t.blocked().len(), 1);
    w.set_status(&falcon, "archived").await;

    let screening = w.flag().await;
    w.confirm_and_approve(&screening).await;

    let omar = w.t.reload(&w.omar).await;
    assert_eq!(omar.status, PartyStatus::Blocked);
    assert_eq!(
        omar.hold_reason.as_deref(),
        Some("non_payment"),
        "unchanged"
    );
    let records = BlockRecord::open_for(w.t.db(), omar.id).await.unwrap();
    let codes: Vec<_> = records.iter().map(|r| r.reason_code.as_str()).collect();
    assert_eq!(codes, ["non_payment", "sanctions"]);
    assert_eq!(w.history_of(&omar).await, ["block"], "no second block");
    assert_eq!(w.t.blocked().len(), 1, "no second PartyBlocked");

    let falcon_after = w.t.reload(&falcon).await;
    assert_eq!(falcon_after.status, PartyStatus::Archived);
    let records = BlockRecord::open_for(w.t.db(), falcon.id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert!(records[0].restricted && records[0].reason_code == "sanctions");
    assert!(w.history_of(&falcon_after).await.is_empty());

    let freeze = w.tasks(&screening).await.remove(0);
    let standing: Vec<_> = freeze
        .parties(w.t.db())
        .await
        .unwrap()
        .iter()
        .map(|p| p.standing)
        .collect();
    assert_eq!(
        standing,
        [PartyStanding::BlockedAlready, PartyStanding::Archived]
    );
}

#[renox::test]
async fn running_the_effects_again_adds_nothing() {
    let w = world().await;
    let falcon = party(&w.t, PartyKind::Company, "Falcon Holdings LLC").await;
    w.relate(&falcon, RelationshipType::UboOf).await;
    let screening = w.flag().await;
    w.confirm_and_approve(&screening).await;

    let stored = Screening::find(w.t.db(), screening.id)
        .await
        .unwrap()
        .unwrap();
    let again = apply_confirmed_match(w.t.app.state(), &stored, Some(w.a.id))
        .await
        .unwrap();
    assert!(!again.first_run);
    for party in [&w.omar, &falcon] {
        assert_eq!(
            BlockRecord::open_for(w.t.db(), party.id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    assert_eq!(w.tasks(&screening).await.len(), 2);
    assert_eq!(w.confirmed.lock().unwrap().len(), 1);
    assert_eq!(w.t.blocked().len(), 2);
    let standing: Vec<_> = again.parties.iter().map(|p| p.standing).collect();
    assert_eq!(standing, [PartyStanding::Blocked, PartyStanding::Blocked]);
    let bells = w.b.unread_notifications(w.t.db()).await.unwrap();
    assert_eq!(
        bells
            .iter()
            .filter_map(|b| b.message())
            .filter(|m| m.title == "Freeze and report tasks opened")
            .count(),
        1
    );
}

#[renox::test]
async fn the_report_task_can_be_due_later_than_the_freeze_task() {
    let w = world_with(WlParties::new().report_task_due_hours(48)).await;
    let screening = w.flag().await;
    w.confirm_and_approve(&screening).await;
    let tasks = w.tasks(&screening).await;
    assert_eq!(
        tasks[1].due_at - tasks[0].due_at,
        renox::chrono::Duration::hours(24)
    );
}

#[renox::test]
async fn only_a_reviewer_may_confirm_and_only_a_possible_match() {
    let w = world().await;
    let screening = w.flag().await;
    let clerk =
        w.t.user("Cara", &[perm::VIEW, perm::BLOCK_REASON_VIEW])
            .await;
    assert_eq!(w.confirm(&clerk, &screening, "Same DOB").await, "403");
    w.confirm_and_approve(&screening).await;
    assert_eq!(w.confirm(&w.a, &screening, "Again").await, "409");
}
