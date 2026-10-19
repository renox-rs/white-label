//! Restoring an archived party (#39 AC5) and the settings of a confirmed
//! match. `archive` does not exist yet, so a test archives a party with a
//! direct SQL update.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use renox::state_machine::{self, TransitionError};
use wl_kernel::{PartyRestored, PartyStatus};
use wl_parties::block_record::add_record_without_transition;
use wl_parties::{BlockRecord, ConfirmedMatchSettings, Party, WlParties, permissions as perm};

use support::{Booted, boot_custom};

const FALCON: &str = "Gulf Build Materials LLC";

/// A test-only route: the signed-in user restores a party (a test cannot
/// build an `AuthUser` itself).
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

struct RestoreRoute;

impl Module for RestoreRoute {
    fn name(&self) -> &'static str {
        "restore-route"
    }

    fn routes(&self) -> Routes {
        Routes::new().post("/t/restore/{party_id}", restore_route)
    }
}

async fn restore(t: &Booted, user: &User, party: &Party) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .post(&format!("/t/restore/{}", party.id), &[("x", "1")])
        .await
}

async fn boot_listening() -> (Booted, Arc<Mutex<Vec<PartyRestored>>>) {
    let restored = Arc::new(Mutex::new(Vec::new()));
    let r = Arc::clone(&restored);
    let t = boot_custom(WlParties::new(), true, move |app| {
        app.module(RestoreRoute)
            .listen(move |event: PartyRestored, _state: AppState| {
                let r = Arc::clone(&r);
                async move {
                    r.lock().unwrap().push(event);
                    Ok(())
                }
            })
    })
    .await;
    (t, restored)
}

async fn archive(t: &Booted, party: &Party) {
    renox::db::sql("UPDATE wl_parties_parties SET status = 'archived' WHERE id = ?")
        .bind(party.id)
        .execute(t.db())
        .await
        .unwrap();
}

#[renox::test]
async fn a_party_without_an_open_record_is_restored_to_active() {
    let (t, restored) = boot_listening().await;
    let party = t.party(FALCON).await;
    archive(&t, &party).await;
    let user = t.user("Rita", &[perm::VIEW, perm::ARCHIVE]).await;

    restore(&t, &user, &party).await.assert_ok();
    let after = t.reload(&party).await;
    assert_eq!(after.status, PartyStatus::Active);
    assert_eq!(after.hold_reason, None);
    assert_eq!(restored.lock().unwrap().len(), 1);
    assert!(t.blocked().is_empty());
    let history = state_machine::history(t.db(), Party::TABLE, party.id)
        .await
        .unwrap();
    assert_eq!(history.last().unwrap().transition, "restore");
}

#[renox::test]
async fn a_party_with_an_open_record_is_restored_to_blocked() {
    let (t, restored) = boot_listening().await;
    let party = t.party(FALCON).await;
    archive(&t, &party).await;
    add_record_without_transition(t.db(), party.id, 5, "sanctions", true, "match", Some(3))
        .await
        .unwrap()
        .expect("added");
    let user = t.user("Rita", &[perm::VIEW, perm::ARCHIVE]).await;

    restore(&t, &user, &party).await.assert_ok();
    let after = t.reload(&party).await;
    assert_eq!(after.status, PartyStatus::Blocked);
    assert_eq!(after.hold_reason.as_deref(), Some("on_hold"));
    assert_eq!(restored.lock().unwrap().len(), 1);
    let blocked = t.blocked();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].display_reason, "on_hold");
    let history = state_machine::history(t.db(), Party::TABLE, party.id)
        .await
        .unwrap();
    let row = history.last().unwrap();
    assert_eq!(row.transition, "restore_blocked");
    assert!(!format!("{row:?}").contains("sanctions"));
    // The record stays open: only `unblock` lifts it.
    let records = BlockRecord::open_for(t.db(), party.id).await.unwrap();
    assert_eq!(records.len(), 1);
}

#[renox::test]
async fn restoring_needs_the_permission_and_an_archived_party() {
    let (t, restored) = boot_listening().await;
    let party = t.party(FALCON).await;
    let without = t.user("Walt", &[perm::VIEW]).await;
    let with = t.user("Rita", &[perm::VIEW, perm::ARCHIVE]).await;

    archive(&t, &party).await;
    restore(&t, &without, &party).await.assert_status(403);
    assert_eq!(t.reload(&party).await.status, PartyStatus::Archived);

    restore(&t, &with, &party).await.assert_ok();
    restore(&t, &with, &party).await.assert_status(409);
    assert_eq!(restored.lock().unwrap().len(), 1);
}

#[renox::test]
async fn the_module_provides_the_confirmed_match_settings() {
    let t = boot_custom(
        WlParties::new()
            .related_company_block(false)
            .report_task_due_hours(48),
        false,
        |app| app,
    )
    .await;
    let settings = ConfirmedMatchSettings::of(t.app.state());
    assert!(!settings.related_company_block);
    assert_eq!(settings.freeze_task_due_hours, 24);
    assert_eq!(settings.report_task_due_hours, 48);
}
