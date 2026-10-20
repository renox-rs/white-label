//! BL-TKT-08 (#153) through the real `wl-parties`: the vendor is blocked by a
//! compliance user through the party's own route, the real block guard
//! refuses a new assignment with the neutral text (no reason), the planned
//! visit of its contract is created unassigned, and unblocking lets the
//! vendor be assigned again.

mod support;

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::tickets::Assignee;
use wl_parties::{Party, WlParties, permissions as party_perm};
use wl_ticketing::{TicketStatus, WlTicketing};

use support::contracts::{World, tower_a_cleaning, world_customized};

async fn real_world() -> World {
    let world = world_customized(
        WlTicketing::new().compliance_manager_role("property_manager"),
        |app| app.module(WlParties::new()),
    )
    .await;
    wl_parties::seed::run(world.t.state())
        .await
        .expect("the demo parties seed");
    world
}

async fn vendor(w: &World) -> PartyId {
    let party = Party::query()
        .where_(Party::LEGAL_NAME_EN.eq("Gulf Build Materials LLC"))
        .first(w.t.db())
        .await
        .unwrap()
        .expect("the demo vendor");
    PartyId(party.id)
}

#[renox::test]
async fn tkt08_the_real_block_stops_new_work_and_the_unblock_restores_it() {
    let w = real_world().await;
    let vendor = vendor(&w).await;
    let contract = w
        .contract(
            vendor,
            "2026-01-01",
            "2099-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    let compliance =
        w.t.user(
            "Mariam",
            &[
                party_perm::VIEW,
                party_perm::BLOCK,
                party_perm::BLOCK_REASON_VIEW,
            ],
        )
        .await;
    let ticket = w.t.open("plumbing", None).await;
    let party = vendor.0.to_string();
    let pairs = [("party", party.as_str()), ("priority", "high")];

    // Not blocked yet: the vendor can be assigned.
    let other = w.t.open("plumbing", None).await;
    w.t.assign(&w.coordinator, &other, "2026-11-03 09:00", &pairs)
        .await
        .assert_ok();

    w.t.app.acting_as(&compliance);
    w.t.app
        .htmx()
        .post(
            &format!("/parties/{}/block", vendor.0),
            &[("reason_code", "fraud_suspected"), ("note", "invoices")],
        )
        .await
        .assert_ok();

    w.t.assign(&w.coordinator, &ticket, "2026-11-03 09:00", &pairs)
        .await
        .assert_status(422)
        .assert_see("This party is on hold")
        .assert_dont_see("fraud_suspected");
    assert_eq!(w.t.reload(&ticket).await.status, TicketStatus::Open);

    // The next planned visit of its contract is created unassigned.
    let visit = w
        .visit(
            &contract,
            "cleaning",
            tower_a_cleaning(),
            "2099-03-01",
            Some(Assignee::party(vendor)),
        )
        .await;
    assert_eq!(visit.status, TicketStatus::Open);
    assert_eq!(visit.assignee(), None);

    // The unblock restores it.
    // Another user lifts it (a user cannot lift their own block).
    let deputy =
        w.t.user(
            "Dina",
            &[party_perm::VIEW, party_perm::BLOCK, party_perm::EXPORT],
        )
        .await;
    w.t.app.acting_as(&deputy);
    w.t.app
        .htmx()
        .post_multipart(
            &format!("/parties/{}/unblock", vendor.0),
            &[("note", "cleared")],
            &[],
        )
        .await
        .assert_ok();
    w.t.assign(&w.coordinator, &ticket, "2026-11-03 09:00", &pairs)
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&ticket).await.assignee_party_id, Some(vendor.0));
}
