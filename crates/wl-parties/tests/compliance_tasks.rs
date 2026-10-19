//! The freeze and report tasks of a confirmed sanctions match (#39, part 3):
//! creation (due from the flag, funds pre-fill, idempotent per screening and
//! kind) and the `complete` machine. The confirmation itself comes later, so
//! a test blocks parties with the helpers the confirmation will use.

mod support;

use renox::chrono::{Duration, TimeZone, Utc};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::held_funds::{FundsFuture, FundsSource, HeldFund, HeldFundsSource};
use wl_kernel::{PartyId, PartyStatus};
use wl_parties::block_record::add_record_without_transition;
use wl_parties::compliance_tasks::{CompleteInput, NamedParty, complete, create_for_screening};
use wl_parties::{
    ComplianceTask, ComplianceTaskKind, ComplianceTaskStatus, ConfirmedMatchSettings, Party,
    PartyStanding, WlParties, permissions as perm,
};
use wl_parties::{Screening, ScreeningTask};

use support::{Booted, boot_custom};

const AL_NOOR: &str = "Al Noor Trading LLC";
const FALCON: &str = "Gulf Build Materials LLC";

/// Reports money for Al Noor only, so other tests in this binary see none.
struct TestFunds;

impl FundsSource for TestFunds {
    fn held_for<'a>(
        &'a self,
        state: &'a AppState,
        party: PartyId,
    ) -> FundsFuture<'a, Vec<HeldFund>> {
        Box::pin(async move {
            let name = Party::find(&state.db, party.0)
                .await?
                .map(|p| p.legal_name_en)
                .unwrap_or_default();
            Ok(if name == AL_NOOR {
                vec![
                    HeldFund::new("Deposit D-1", "deposit", 150_000, "AED"),
                    HeldFund::new("Refund R-7", "refund", 25_050, "AED"),
                ]
            } else {
                Vec::new()
            })
        })
    }
}

/// A test-only route: the signed-in user completes a task. `mode` picks the
/// input: `freeze`, `report` or `none`.
async fn complete_route(
    State(state): State<AppState>,
    user: AuthUser,
    Path((task_id, mode)): Path<(i64, String)>,
) -> Result<Response> {
    let now = renox::db::now();
    let input = match mode.as_str() {
        "freeze" => CompleteInput {
            frozen_at: Some(now),
            ..CompleteInput::default()
        },
        "report" => CompleteInput {
            reference: Some(" GOAML-2026-17 ".into()),
            reported_on: Some(now),
            ..CompleteInput::default()
        },
        _ => CompleteInput::default(),
    };
    match complete(&state, &user, task_id, &input).await {
        Ok(()) => Ok("done".into_response()),
        Err(TransitionError::Forbidden) => Ok(StatusCode::FORBIDDEN.into_response()),
        Err(TransitionError::Guard(_)) => Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response()),
        Err(TransitionError::NotFrom(_)) => Ok(StatusCode::CONFLICT.into_response()),
        Err(other) => Err(Error::permanent_message(other)),
    }
}

struct CompleteRoute;

impl Module for CompleteRoute {
    fn name(&self) -> &'static str {
        "complete-route"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/t/complete/{task_id}/{mode}", complete_route)
    }
}

async fn boot_with_settings(settings: ConfirmedMatchSettings) -> Booted {
    let module = WlParties::new()
        .freeze_task_due_hours(settings.freeze_task_due_hours)
        .report_task_due_hours(settings.report_task_due_hours);
    boot_custom(module, true, |app| app.module(CompleteRoute)).await
}

async fn boot() -> Booted {
    boot_with_settings(ConfirmedMatchSettings::DEFAULT).await
}

fn flagged_at() -> DateTime {
    Utc.with_ymd_and_hms(2026, 11, 3, 10, 0, 0).unwrap()
}

/// A screening of `party` linked to a review task flagged at [`flagged_at`].
async fn screening_of(t: &Booted, party: &Party) -> Screening {
    let mut task = ScreeningTask {
        party_id: party.id,
        flagged_at: flagged_at(),
        due_at: flagged_at() + Duration::hours(24),
        ..ScreeningTask::default()
    };
    task.insert(t.db()).await.unwrap();
    let mut screening = Screening {
        party_id: party.id,
        run_at: flagged_at(),
        task_id: Some(task.id),
        ..Screening::default()
    };
    screening.insert(t.db()).await.unwrap();
    screening
}

/// Blocks `party` the way a confirmation does: status and a restricted record.
async fn freeze_party(
    t: &Booted,
    party: &Party,
    screening: &Screening,
    status: &str,
) -> NamedParty {
    renox::db::sql("UPDATE wl_parties_parties SET status = ? WHERE id = ?")
        .bind(status)
        .bind(party.id)
        .execute(t.db())
        .await
        .unwrap();
    let record =
        add_record_without_transition(t.db(), party.id, screening.id, "sanctions", true, "m", None)
            .await
            .unwrap()
            .expect("added");
    NamedParty {
        party_id: party.id,
        block_record_id: Some(record.id),
        standing: if status == "archived" {
            PartyStanding::Archived
        } else {
            PartyStanding::Blocked
        },
    }
}

async fn create(
    t: &Booted,
    screening: &Screening,
    parties: &[NamedParty],
) -> wl_parties::compliance_tasks::ComplianceTasks {
    create_for_screening(t.app.state(), screening, parties)
        .await
        .unwrap()
}

fn named(party: &Party) -> NamedParty {
    NamedParty {
        party_id: party.id,
        block_record_id: None,
        standing: PartyStanding::Blocked,
    }
}

#[renox::test]
async fn both_tasks_are_due_24_hours_after_the_flag_and_name_every_party() {
    let t = boot().await;
    let (omar, falcon) = (t.party(AL_NOOR).await, t.party(FALCON).await);
    let screening = screening_of(&t, &omar).await;
    let tasks = create(&t, &screening, &[named(&omar), named(&falcon)]).await;

    assert_eq!(tasks.freeze.kind, ComplianceTaskKind::Freeze);
    assert_eq!(tasks.report.kind, ComplianceTaskKind::Report);
    let due = flagged_at() + Duration::hours(24);
    assert_eq!(tasks.freeze.due_at, due);
    assert_eq!(tasks.report.due_at, due);
    assert_eq!(tasks.freeze.status, ComplianceTaskStatus::Open);
    for task in [&tasks.freeze, &tasks.report] {
        let ids: Vec<i64> = task
            .parties(t.db())
            .await
            .unwrap()
            .iter()
            .map(|p| p.party_id)
            .collect();
        assert_eq!(ids, vec![omar.id, falcon.id]);
    }
}

#[renox::test]
async fn the_due_hours_come_from_the_settings_and_a_late_task_is_overdue() {
    let t = boot_with_settings(ConfirmedMatchSettings {
        report_task_due_hours: 48,
        ..ConfirmedMatchSettings::DEFAULT
    })
    .await;
    let omar = t.party(AL_NOOR).await;
    let screening = screening_of(&t, &omar).await;
    let tasks = create(&t, &screening, &[named(&omar)]).await;

    assert_eq!(tasks.freeze.due_at, flagged_at() + Duration::hours(24));
    assert_eq!(tasks.report.due_at, flagged_at() + Duration::hours(48));
    // The flag was in 2026-11; today is after that, so no clamp: overdue.
    let now = flagged_at() + Duration::hours(30);
    assert!(tasks.freeze.is_overdue(now));
    assert!(!tasks.report.is_overdue(now));
}

#[renox::test]
async fn creating_twice_adds_nothing() {
    let t = boot().await;
    let omar = t.party(AL_NOOR).await;
    let screening = screening_of(&t, &omar).await;
    let first = create(&t, &screening, &[named(&omar)]).await;
    let second = create(&t, &screening, &[named(&omar)]).await;

    assert_eq!(first.freeze.id, second.freeze.id);
    assert_eq!(first.report.id, second.report.id);
    let all = ComplianceTask::for_screening(t.db(), screening.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(first.freeze.parties(t.db()).await.unwrap().len(), 1);
}

#[renox::test]
async fn the_freeze_task_is_pre_filled_with_unconfirmed_funds() {
    HeldFundsSource::new("test-deposits")
        .owner("wl-parties-tests")
        .source(TestFunds)
        .register();
    let t = boot().await;
    let (omar, falcon) = (t.party(AL_NOOR).await, t.party(FALCON).await);
    let screening = screening_of(&t, &omar).await;
    let tasks = create(&t, &screening, &[named(&omar), named(&falcon)]).await;

    let funds = tasks.freeze.funds(t.db()).await.unwrap();
    assert_eq!(funds.len(), 2);
    assert_eq!(funds[0].label, "Deposit D-1");
    assert_eq!(funds[0].amount_fils, 150_000);
    assert_eq!(funds[0].currency, "AED");
    assert_eq!(funds[0].party_id, Some(omar.id));
    assert_eq!(funds[0].source_key, "test-deposits");
    assert!(funds.iter().all(|f| !f.confirmed));
    assert!(tasks.report.funds(t.db()).await.unwrap().is_empty());

    // A rerun does not fill it twice.
    create(&t, &screening, &[named(&omar), named(&falcon)]).await;
    assert_eq!(tasks.freeze.funds(t.db()).await.unwrap().len(), 2);
}

#[renox::test]
async fn a_freeze_task_is_done_only_with_every_party_frozen_and_a_time() {
    let t = boot().await;
    let (omar, falcon) = (t.party(AL_NOOR).await, t.party(FALCON).await);
    let screening = screening_of(&t, &omar).await;
    let tasks = create(&t, &screening, &[named(&omar), named(&falcon)]).await;
    let reviewer = t
        .user(
            "Rana",
            &[perm::VIEW, perm::BLOCK_REASON_VIEW, perm::SCREENING_REVIEW],
        )
        .await;
    t.app.acting_as(&reviewer);
    let path = |mode: &str| format!("/t/complete/{}/{mode}", tasks.freeze.id);

    // Nobody is blocked yet.
    t.app
        .post(&path("freeze"), &[("x", "1")])
        .await
        .assert_status(422);

    freeze_party(&t, &omar, &screening, "blocked").await;
    t.app
        .post(&path("freeze"), &[("x", "1")])
        .await
        .assert_status(422);

    // An archived party with an open record counts.
    freeze_party(&t, &falcon, &screening, "archived").await;
    // No freeze time.
    t.app
        .post(&path("none"), &[("x", "1")])
        .await
        .assert_status(422);
    t.app.post(&path("freeze"), &[("x", "1")]).await.assert_ok();

    let done = ComplianceTask::find(t.db(), tasks.freeze.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, ComplianceTaskStatus::Done);
    assert_eq!(done.completed_by, Some(reviewer.id));
    assert!(done.frozen_at.is_some());
    assert!(done.completed_at.is_some());
    // Done is final.
    t.app
        .post(&path("freeze"), &[("x", "1")])
        .await
        .assert_status(409);
    assert_eq!(t.reload(&omar).await.status, PartyStatus::Blocked);
}

#[renox::test]
async fn a_report_task_stores_the_reference_and_date() {
    let t = boot().await;
    let omar = t.party(AL_NOOR).await;
    let screening = screening_of(&t, &omar).await;
    let tasks = create(&t, &screening, &[named(&omar)]).await;
    let reviewer = t
        .user(
            "Rana",
            &[perm::VIEW, perm::BLOCK_REASON_VIEW, perm::SCREENING_REVIEW],
        )
        .await;
    t.app.acting_as(&reviewer);
    let path = |mode: &str| format!("/t/complete/{}/{mode}", tasks.report.id);

    t.app
        .post(&path("none"), &[("x", "1")])
        .await
        .assert_status(422);
    t.app.post(&path("report"), &[("x", "1")]).await.assert_ok();

    let done = ComplianceTask::find(t.db(), tasks.report.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, ComplianceTaskStatus::Done);
    assert_eq!(done.reference.as_deref(), Some("GOAML-2026-17"));
    assert!(done.reported_on.is_some());
}

#[renox::test]
async fn completing_needs_both_permissions() {
    let t = boot().await;
    let omar = t.party(AL_NOOR).await;
    let screening = screening_of(&t, &omar).await;
    let tasks = create(&t, &screening, &[named(&omar)]).await;
    let path = format!("/t/complete/{}/report", tasks.report.id);

    // Can read the tasks, cannot complete them.
    let reader = t
        .user("Reema", &[perm::VIEW, perm::BLOCK_REASON_VIEW])
        .await;
    t.app.acting_as(&reader);
    t.app.post(&path, &[("x", "1")]).await.assert_status(403);
    // Reviews screenings, but is not a holder of the restricted view.
    let partner = t.user("Pia", &[perm::VIEW, perm::SCREENING_REVIEW]).await;
    t.app.acting_as(&partner);
    t.app.post(&path, &[("x", "1")]).await.assert_status(403);

    let task = ComplianceTask::find(t.db(), tasks.report.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.status, ComplianceTaskStatus::Open);
}
