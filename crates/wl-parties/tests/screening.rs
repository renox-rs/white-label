//! The screening base (wl-parties §4, issue #38 part 2): `record_run` picks
//! `complete_clear` or `flag` by the match threshold.

mod support;

use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::ScreeningStatus;
use wl_parties::{
    NewCandidate, Party, Screening, ScreeningRun, ScreeningTrigger, WlParties, permissions as perm,
    record_run,
};

use support::{Booted, boot, boot_custom, boot_with};

const AL_NOOR: &str = "Al Noor Trading LLC";

fn candidate(score: f64) -> NewCandidate {
    NewCandidate {
        name: "Omar K.".into(),
        list_name: "UN consolidated".into(),
        score,
        list_entry: Some("QDi.001".into()),
    }
}

fn automatic(scores: &[f64]) -> ScreeningRun {
    ScreeningRun::automatic(
        ScreeningTrigger::Onboarding,
        scores.iter().map(|s| candidate(*s)).collect(),
    )
}

async fn screenings(t: &Booted, party: &Party) -> Vec<Screening> {
    Screening::for_party(t.db(), party.id).await.unwrap()
}

#[renox::test]
async fn with_screening_off_a_run_is_refused_and_nothing_is_stored() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let refused = record_run(t.app.state(), &party, automatic(&[0.99]), None).await;
    assert!(matches!(refused, Err(TransitionError::Guard(_))));
    assert!(screenings(&t, &party).await.is_empty());
}

#[renox::test]
async fn a_run_below_the_threshold_is_clear_and_at_it_a_possible_match() {
    let t = boot_with(WlParties::new().screening(true), true).await;
    let party = t.party(AL_NOOR).await;
    let clear = record_run(t.app.state(), &party, automatic(&[0.84, 0.2]), None)
        .await
        .unwrap();
    assert_eq!(clear.status, ScreeningStatus::Clear);
    assert_eq!(clear.candidates(t.db()).await.unwrap().len(), 2);
    let none = record_run(t.app.state(), &party, automatic(&[]), None)
        .await
        .unwrap();
    assert_eq!(none.status, ScreeningStatus::Clear);
    let flagged = record_run(t.app.state(), &party, automatic(&[0.85]), None)
        .await
        .unwrap();
    assert_eq!(flagged.status, ScreeningStatus::PossibleMatch);
    assert_eq!(screenings(&t, &party).await.len(), 3);
    // The party is never blocked by a flag.
    assert_eq!(
        t.reload(&party).await.status,
        wl_kernel::PartyStatus::Active
    );
}

#[renox::test]
async fn the_threshold_is_a_setting() {
    let module = WlParties::new().screening(true).match_threshold(0.9);
    let t = boot_with(module, true).await;
    let party = t.party(AL_NOOR).await;
    let below = record_run(t.app.state(), &party, automatic(&[0.89]), None)
        .await
        .unwrap();
    assert_eq!(below.status, ScreeningStatus::Clear);
    let at = record_run(t.app.state(), &party, automatic(&[0.9]), None)
        .await
        .unwrap();
    assert_eq!(at.status, ScreeningStatus::PossibleMatch);
}

#[renox::test]
async fn a_manual_flag_needs_a_reviewer_and_the_list_entry() {
    let module = WlParties::new().screening(true);
    let t = boot_custom(module, true, |app| app.module(ManualChecks)).await;
    let party = t.party(AL_NOOR).await;
    let reviewer = t.user("Rana", &[perm::VIEW, perm::SCREENING_REVIEW]).await;
    let viewer = t.user("Vik", &[perm::VIEW]).await;
    let url = format!("/t/screening/manual/{}", party.id);

    t.app.acting_as(&reviewer);
    t.app.post(&url, &[("entry", "")]).await.assert_status(422);
    t.app.acting_as(&viewer);
    t.app
        .post(&url, &[("entry", "QDi.001")])
        .await
        .assert_status(422);
    assert!(screenings(&t, &party).await.is_empty());

    t.app.acting_as(&reviewer);
    t.app
        .post(&url, &[("entry", "QDi.001")])
        .await
        .assert_ok()
        .assert_see("PossibleMatch");
    let stored = screenings(&t, &party).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].status, ScreeningStatus::PossibleMatch);
    assert_eq!(stored[0].reviewer_user, Some(reviewer.id));
}

/// A test-only route: the signed-in user records a manual check (a test
/// cannot build an `AuthUser` itself).
async fn manual(
    State(state): State<AppState>,
    user: AuthUser,
    Path(party_id): Path<i64>,
    Form(form): Form<ManualForm>,
) -> Result<Response> {
    let party = Party::find(&state.db, party_id)
        .await?
        .ok_or(Error::NotFound)?;
    let mut candidate = candidate(0.95);
    candidate.list_entry = Some(form.entry);
    let run = ScreeningRun::manual(vec![candidate]);
    match record_run(&state, &party, run, Some(&user)).await {
        Ok(screening) => Ok(format!("{:?}", screening.status).into_response()),
        Err(TransitionError::Guard(message)) => {
            Ok((StatusCode::UNPROCESSABLE_ENTITY, message).into_response())
        }
        Err(other) => Err(Error::permanent_message(other)),
    }
}

#[derive(serde::Deserialize)]
struct ManualForm {
    entry: String,
}

struct ManualChecks;

impl Module for ManualChecks {
    fn name(&self) -> &'static str {
        "manual-checks"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/t/screening/manual/{party_id}", manual)
    }
}

#[renox::test]
async fn a_status_changed_outside_a_transition_is_refused() {
    let t = boot_with(WlParties::new().screening(true), true).await;
    let party = t.party(AL_NOOR).await;
    let mut screening = record_run(t.app.state(), &party, automatic(&[0.1]), None)
        .await
        .unwrap();
    screening.status = ScreeningStatus::ConfirmedMatch;
    assert!(screening.save(t.db()).await.is_err());
    let stored = Screening::find(t.db(), screening.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ScreeningStatus::Clear);
}

#[renox::test]
async fn a_bad_candidate_score_is_refused() {
    let t = boot_with(WlParties::new().screening(true), true).await;
    let party = t.party(AL_NOOR).await;
    let refused = record_run(t.app.state(), &party, automatic(&[1.5]), None).await;
    assert!(matches!(refused, Err(TransitionError::Guard(_))));
    assert!(screenings(&t, &party).await.is_empty());
}
