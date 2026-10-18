//! The module's block guard (wl-parties §6, #37 part 4): which parties it
//! refuses, what each viewer sees, and who approves an exception.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::block_guard::{ActionKind, BlockRefusal, GuardOutcome, guard};
use wl_parties::{BlockRecord, WlParties, permissions as perm};

use support::{Booted, block_party, boot, boot_custom, boot_with};

/// A test-only route that asks the guard as the signed-in user, so that the
/// user's permissions are those of the request (as in a module's handler).
struct Probe;

impl Module for Probe {
    fn name(&self) -> &'static str {
        "guard-probe"
    }

    fn routes(&self) -> Routes {
        Routes::new().get("/guard-probe/{id}", probe).require_auth()
    }
}

async fn probe(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Option<BlockRefusal>>> {
    let mut tx = state.db.begin().await?;
    let outcome = guard(&state)
        .check_in(&mut tx, Some(&user), PartyId(id), ActionKind::MoneyOut)
        .await?;
    Ok(Json(match outcome {
        GuardOutcome::Allowed => None,
        GuardOutcome::Blocked(refusal) => Some(refusal),
    }))
}

/// The refusal `user` gets for `party` through the probe route.
async fn refusal_of(t: &Booted, user: &User, party: i64) -> BlockRefusal {
    t.app.acting_as(user);
    let response = t.app.get(&format!("/guard-probe/{party}")).await;
    response.assert_ok();
    response.json::<Option<BlockRefusal>>().expect("refused")
}

const AL_NOOR: &str = "Al Noor Trading LLC";

async fn check(t: &Booted, viewer: Option<&User>, party: i64, kind: ActionKind) -> GuardOutcome {
    let guard = guard(t.app.state());
    let mut tx = t.app.state().db.begin().await.unwrap();
    guard
        .check_in(&mut tx, viewer, PartyId(party), kind)
        .await
        .unwrap()
}

#[renox::test]
async fn an_active_or_unknown_party_is_allowed() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    for kind in ActionKind::ALL {
        assert!(check(&t, None, party.id, kind).await.is_allowed(), "{kind}");
        assert!(check(&t, None, 999_999, kind).await.is_allowed(), "{kind}");
    }
}

#[renox::test]
async fn a_blocked_party_is_refused_except_for_money_in() {
    let t = boot().await;
    let party = t.party(AL_NOOR).await;
    let user = t.user_a().await;
    block_party(&t, &user, &party, "non_payment", "Unpaid").await;
    for kind in ActionKind::ALL {
        let outcome = check(&t, Some(&user), party.id, kind).await;
        assert_eq!(outcome.is_allowed(), kind == ActionKind::MoneyIn, "{kind}");
    }
}

#[renox::test]
async fn the_refusal_is_neutral_without_the_permission_and_detailed_with_it() {
    let t = boot_custom(WlParties::new(), true, |app| app.module(Probe)).await;
    let party = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    block_party(&t, &mlro, &party, "sanctions", "Listed").await;
    let user_b = t.user_b().await;

    let neutral = refusal_of(&t, &user_b, party.id).await;
    assert_eq!(neutral.code(), "on_hold");
    assert_eq!(neutral.message_en(), "This party is on hold");
    assert!(neutral.record_links().is_empty());
    assert_eq!(neutral.approver, "Finance manager");

    let GuardOutcome::Blocked(system) = check(&t, None, party.id, ActionKind::Start).await else {
        panic!("refused");
    };
    assert!(!system.is_detailed());

    let detailed = refusal_of(&t, &mlro, party.id).await;
    assert_eq!(detailed.message_en(), "Blocked: Sanctions");
    let records = BlockRecord::open_for(t.db(), party.id).await.unwrap();
    assert_eq!(
        detailed.record_links(),
        [format!(
            "/parties/{}/block-records/{}",
            party.id, records[0].id
        )]
    );
}

#[renox::test]
async fn the_exception_step_follows_screening_and_the_override() {
    let default = boot().await;
    let step = guard(default.app.state()).exception_step();
    assert_eq!(step.role, "finance_manager");
    assert_eq!(step.permission, perm::BLOCK_EXCEPTION);

    let screening = boot_with(WlParties::new().screening(true), false).await;
    assert_eq!(
        guard(screening.app.state()).exception_step().role,
        "compliance"
    );

    let custom = boot_with(WlParties::new().exception_approver("cfo"), false).await;
    assert_eq!(guard(custom.app.state()).exception_step().role, "cfo");
}
