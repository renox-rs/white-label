//! The `record_manual_check` service (wl-parties §4, issue #42 part 3): the
//! permission, the refusals, and the Screening it records. A test-only route
//! calls it as the signed-in user (`acting_as`). The full acceptance checks
//! are in part 5.

mod support;

use renox::prelude::*;
use serde::Deserialize;
use wl_kernel::ScreeningStatus;
use wl_parties::{
    ListFormat, ListStatus, ManualCheckError, ManualResult, Party, SanctionsList, Screening,
    ScreeningMethod, ScreeningTask, ScreeningTrigger, WlParties, permissions as perm,
    record_manual_check,
};

use support::{Booted, archive_party, boot_custom, set_dubai_clock};

const PDF: &str = "Local list (PDF)";

#[derive(Deserialize)]
struct CheckForm {
    list_id: i64,
    list_date: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    entry: String,
}

async fn check_route(
    State(state): State<AppState>,
    user: AuthUser,
    Path(party_id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response> {
    let party = Party::find(&state.db, party_id)
        .await?
        .ok_or(Error::NotFound)?;
    let result = if form.name.is_empty() && form.entry.is_empty() {
        ManualResult::NoMatch
    } else {
        ManualResult::Candidate {
            name: form.name,
            list_entry: form.entry,
        }
    };
    match record_manual_check(&state, &user, &party, form.list_id, &form.list_date, result).await {
        Ok(s) => Ok(format!("{:?}", s.status).into_response()),
        Err(ManualCheckError::Forbidden) => Ok(StatusCode::FORBIDDEN.into_response()),
        Err(ManualCheckError::NotFound) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(ManualCheckError::Refused(why)) => {
            Ok((StatusCode::UNPROCESSABLE_ENTITY, why.to_string()).into_response())
        }
        Err(ManualCheckError::Run(e)) => Err(Error::permanent_message(e)),
    }
}

struct CheckRoute;

impl Module for CheckRoute {
    fn name(&self) -> &'static str {
        "check-route"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/t/parties/{party_id}/check", check_route)
    }
}

async fn setup(screening: bool) -> (Booted, SanctionsList) {
    let t = boot_custom(WlParties::new().screening(screening), true, |app| {
        app.module(CheckRoute)
    })
    .await;
    // Today is 2026-11-05 in Dubai.
    set_dubai_clock(&t, 5, 10);
    t.app.fake_events();
    let list = list_of(&t, PDF, ListFormat::Pdf).await;
    (t, list)
}

async fn list_of(t: &Booted, name: &str, format: ListFormat) -> SanctionsList {
    let mut list = SanctionsList {
        name: name.into(),
        source: "https://example.org".into(),
        format,
        status: ListStatus::Current,
        ..SanctionsList::default()
    };
    list.insert(t.db()).await.unwrap();
    list
}

async fn reviewer(t: &Booted) -> User {
    t.user("Mariam", &[perm::VIEW, perm::SCREENING_REVIEW])
        .await
}

async fn post(
    t: &Booted,
    user: &User,
    party: &Party,
    list: &SanctionsList,
    date: &str,
    name: &str,
    entry: &str,
) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .post(
            &format!("/t/parties/{}/check", party.id),
            &[
                ("list_id", list.id.to_string().as_str()),
                ("list_date", date),
                ("name", name),
                ("entry", entry),
            ],
        )
        .await
}

async fn screenings(t: &Booted, party: &Party) -> Vec<Screening> {
    Screening::for_party(t.db(), party.id).await.unwrap()
}

#[renox::test]
async fn no_match_records_a_clear_manual_screening() {
    let (t, list) = setup(true).await;
    let user = reviewer(&t).await;
    let party = t.party("Al Noor Trading LLC").await;
    post(&t, &user, &party, &list, "2026-11-03", "", "")
        .await
        .assert_ok();
    let all = screenings(&t, &party).await;
    assert_eq!(all.len(), 1);
    let s = &all[0];
    assert_eq!(s.status, ScreeningStatus::Clear);
    assert_eq!(s.method, ScreeningMethod::Manual);
    assert_eq!(s.run_trigger, ScreeningTrigger::Manual);
    assert_eq!(s.reviewer_user, Some(user.id));
    assert!(!s.lists_stale);
    let used = &s.lists_used.0[0];
    assert_eq!(used.list_id, list.id);
    assert_eq!(used.version.as_deref(), Some("2026-11-03"));
    assert!(!used.stale);
    assert!(ScreeningTask::open_tasks(t.db()).await.unwrap().is_empty());
}

#[renox::test]
async fn a_candidate_flags_the_screening_and_opens_the_task() {
    let (t, list) = setup(true).await;
    let user = reviewer(&t).await;
    let party = t.party("Al Noor Trading LLC").await;
    post(&t, &user, &party, &list, "2026-11-03", "Omar K.", "UAE-12")
        .await
        .assert_ok();
    let s = &screenings(&t, &party).await[0];
    assert_eq!(s.status, ScreeningStatus::PossibleMatch);
    assert!(s.task_id.is_some());
    let tasks = ScreeningTask::open_tasks(t.db()).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].due_at,
        tasks[0].flagged_at + renox::chrono::Duration::hours(24)
    );
    let candidates = s.candidates(t.db()).await.unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].list_entry.as_deref(), Some("UAE-12"));
    assert_eq!(candidates[0].list_name, PDF);
    // A second flag links to the same open task.
    post(&t, &user, &party, &list, "2026-11-04", "Omar K.", "UAE-12")
        .await
        .assert_ok();
    assert_eq!(ScreeningTask::open_tasks(t.db()).await.unwrap().len(), 1);
}

#[renox::test]
async fn a_candidate_without_a_list_entry_is_refused() {
    let (t, list) = setup(true).await;
    let user = reviewer(&t).await;
    let party = t.party("Al Noor Trading LLC").await;
    let response = post(&t, &user, &party, &list, "2026-11-03", "Omar K.", " ").await;
    response.assert_status(422);
    assert!(screenings(&t, &party).await.is_empty());
}

#[renox::test]
async fn a_user_without_screening_review_is_forbidden() {
    let (t, list) = setup(true).await;
    let user = t.user("Dina", &[perm::VIEW, perm::BLOCK_REASON_VIEW]).await;
    let party = t.party("Al Noor Trading LLC").await;
    let response = post(&t, &user, &party, &list, "2026-11-03", "", "").await;
    response.assert_status(403);
    assert!(screenings(&t, &party).await.is_empty());
}

#[renox::test]
async fn refusals_store_nothing() {
    let (t, list) = setup(true).await;
    let user = reviewer(&t).await;
    let party = t.party("Al Noor Trading LLC").await;
    let xml = list_of(&t, "Local list (XML)", ListFormat::Xml).await;
    for (list, date) in [
        (&xml, "2026-11-03"),
        (&list, ""),
        (&list, "2026-11-06"),
        (&list, "yesterday"),
    ] {
        let response = post(&t, &user, &party, list, date, "", "").await;
        response.assert_status(422);
    }
    archive_party(&t, &party).await;
    let response = post(&t, &user, &party, &list, "2026-11-03", "", "").await;
    response.assert_status(422);
    assert!(screenings(&t, &party).await.is_empty());
}

#[renox::test]
async fn screening_off_refuses_the_check() {
    let (t, list) = setup(false).await;
    let user = reviewer(&t).await;
    let party = t.party("Al Noor Trading LLC").await;
    let response = post(&t, &user, &party, &list, "2026-11-03", "", "").await;
    response.assert_status(422);
    assert!(screenings(&t, &party).await.is_empty());
}
