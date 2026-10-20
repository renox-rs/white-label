//! BL-TKT-08 (#153): a blocked vendor's service contracts are under review:
//! no new visit is assigned to it, its planned visits are listed for the
//! facility manager to reassign, and ending is the manager's `terminate`.
//! Nothing ends or is cancelled by itself.

mod support;

use wl_kernel::block_effects::{Decision, source_for};
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_ticketing::vendor_block;
use wl_ticketing::{
    ContractStatus, ServiceContract, ServiceContracts, Ticket, TicketStatus, Tickets,
};

use support::blocks::{Blocks, REAL_REASON};
use support::contracts::{World, tower_a_cleaning, world_with_blocks};
use support::{COOLAIR, dubai};

/// AMC-2026-0003: CoolAir's cleaning contract with 4 planned visits (assigned
/// to it), well in the future.
async fn contract_with_visits(w: &World) -> (ServiceContract, Vec<Ticket>) {
    let contract = w
        .contract(
            COOLAIR,
            "2026-01-01",
            "2099-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    let mut visits = Vec::new();
    for due in ["2099-03-01", "2099-06-01", "2099-09-01", "2099-12-01"] {
        visits.push(
            w.visit(
                &contract,
                "cleaning",
                tower_a_cleaning(),
                due,
                Some(Assignee::party(COOLAIR)),
            )
            .await,
        );
    }
    (contract, visits)
}

/// AC1: with CoolAir blocked for `fraud_suspected`, the facility manager's
/// task lists the contract with its 4 planned visits, and its page offers
/// "Reassign" for each and "Terminate contract"; the reason is nowhere.
#[renox::test]
async fn tkt08_the_review_lists_the_contract_and_the_page_offers_reassign_and_terminate() {
    let (w, blocks) = world_with_blocks().await;
    let (contract, visits) = contract_with_visits(&w).await;
    blocks.block(COOLAIR);

    let source = source_for(vendor_block::KIND).expect("the kind has a source");
    let refs = source.open_for(w.t.state(), COOLAIR).await.unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].subject, contract.subject());
    assert!(refs[0].label.contains(&contract.number));
    assert!(refs[0].label.contains("4 planned visits"));
    assert!(!refs[0].label.contains(REAL_REASON));

    let facility =
        w.t.user(
            "Fiona",
            &[
                wl_ticketing::permissions::VIEW,
                wl_ticketing::permissions::ASSIGN,
                wl_ticketing::permissions::SERVICE_CONTRACTS_VIEW,
                wl_ticketing::permissions::SERVICE_CONTRACTS_MANAGE,
            ],
        )
        .await;
    w.t.app.acting_as(&facility);
    let page =
        w.t.app
            .get(&format!("/service-contracts/{}", contract.id))
            .await;
    page.assert_ok()
        .assert_see("Vendor on hold")
        .assert_see(&format!("terminate-contract-{}", contract.id))
        .assert_dont_see(REAL_REASON);
    for visit in &visits {
        page.assert_see(&format!("reassign-visit-{}", visit.id));
    }

    // Nothing ended by itself.
    assert_eq!(w.reload(&contract).await.status, ContractStatus::Active);
    for visit in &visits {
        assert_eq!(w.t.reload(visit).await.status, TicketStatus::Assigned);
    }
}

/// AC2: assigning a new ticket to CoolAir is refused, without the reason; the
/// next planned visit is created unassigned; `assign` does not propose it.
#[renox::test]
async fn tkt08_a_blocked_vendor_takes_no_new_visit() {
    let (w, blocks) = world_with_blocks().await;
    let (contract, _visits) = contract_with_visits(&w).await;
    blocks.block(COOLAIR);

    let ticket = w.t.open("plumbing", None).await;
    let party = COOLAIR.0.to_string();
    w.t.assign(
        &w.coordinator,
        &ticket,
        "2026-11-03 09:00",
        &[("party", party.as_str()), ("priority", "high")],
    )
    .await
    .assert_status(422)
    .assert_see("This party is on hold")
    .assert_dont_see(REAL_REASON);
    let after = w.t.reload(&ticket).await;
    assert_eq!(after.status, TicketStatus::Open);
    assert_eq!(after.assignee(), None);

    // A reactive ticket opened for it is refused the same way.
    let refused = Tickets::open_at(
        w.t.state(),
        OpenTicket::new(tower_a_cleaning(), "plumbing", "Leak").assign_to(Assignee::party(COOLAIR)),
        dubai("2026-11-03", 8, 0),
    )
    .await;
    assert!(refused.is_err());

    // The next planned visit is created unassigned.
    let next = w
        .visit(
            &contract,
            "cleaning",
            tower_a_cleaning(),
            "2100-03-01",
            Some(Assignee::party(COOLAIR)),
        )
        .await;
    assert_eq!(next.status, TicketStatus::Open);
    assert_eq!(next.assignee(), None);
    assert_eq!(next.contract_id, Some(contract.id));

    // The contract's vendor is not proposed.
    let proposed = ServiceContracts::proposed_vendor(w.t.state(), &next, dubai_date("2026-11-03"))
        .await
        .unwrap();
    assert_eq!(proposed, None);
    blocks.unblock(COOLAIR);
    let proposed = ServiceContracts::proposed_vendor(w.t.state(), &next, dubai_date("2026-11-03"))
        .await
        .unwrap();
    assert_eq!(proposed, Some(COOLAIR));
}

fn dubai_date(text: &str) -> renox::chrono::NaiveDate {
    text.parse().expect("a date")
}

/// AC3: a visit in progress by CoolAir can still be resolved; reassigning a
/// visit away from it checks only the new assignee.
#[renox::test]
async fn tkt08_a_started_visit_continues_and_a_reassignment_away_is_allowed() {
    let (w, blocks) = world_with_blocks().await;
    let (_contract, visits) = contract_with_visits(&w).await;
    let started = w.start(&visits[0]).await;
    blocks.block(COOLAIR);

    w.t.run(&w.coordinator, &started, "resolve", "2026-12-10 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&started).await.status, TicketStatus::Resolved);

    let imran = w.imran.id.to_string();
    w.t.assign(
        &w.coordinator,
        &visits[1],
        "2026-12-10 11:00",
        &[("user", imran.as_str()), ("reason_code", "workload")],
    )
    .await
    .assert_ok();
    assert_eq!(
        w.t.reload(&visits[1]).await.assignee_user_id,
        Some(w.imran.id)
    );

    // But not the other way round: to a blocked vendor.
    let party = COOLAIR.0.to_string();
    w.t.assign(
        &w.coordinator,
        &w.t.reload(&visits[1]).await,
        "2026-12-10 12:00",
        &[("party", party.as_str()), ("reason_code", "workload")],
    )
    .await
    .assert_status(422)
    .assert_see("This party is on hold");
}

/// The decider's choices on the review: pause or continue change nothing; end
/// terminates the contract today, with a neutral reason, and the history
/// names who decided.
#[renox::test]
async fn tkt08_the_decision_to_end_terminates_the_contract_and_the_others_change_nothing() {
    let (w, blocks) = world_with_blocks().await;
    let (contract, visits) = contract_with_visits(&w).await;
    blocks.block(COOLAIR);
    let source = source_for(vendor_block::KIND).expect("a source");
    let subject = contract.subject();

    for decision in [Decision::Pause, Decision::Continue] {
        source
            .decide(w.t.state(), w.admin.id, &subject, decision)
            .await
            .unwrap();
        assert_eq!(w.reload(&contract).await.status, ContractStatus::Active);
    }

    source
        .decide(w.t.state(), w.admin.id, &subject, Decision::End)
        .await
        .unwrap();
    let ended = w.reload(&contract).await;
    assert_eq!(ended.status, ContractStatus::Terminated);
    assert_eq!(ended.termination_reason.as_deref(), Some("Vendor on hold"));
    assert!(!ended.termination_reason.unwrap().contains(REAL_REASON));
    assert!(
        ended.ended_at.is_some(),
        "a termination dated today runs at once"
    );
    assert_eq!(w.ended.all().len(), 1);
    // The visits far in the future are cancelled by the end, as for any
    // termination.
    for visit in &visits {
        assert_eq!(w.t.reload(visit).await.status, TicketStatus::Cancelled);
    }

    // Nobody can end it twice.
    assert!(
        source
            .decide(w.t.state(), w.admin.id, &subject, Decision::End)
            .await
            .is_err()
    );
}

/// On unblock the vendor can be assigned again.
#[renox::test]
async fn tkt08_after_the_unblock_the_vendor_can_be_assigned_again() {
    let (w, blocks) = world_with_blocks().await;
    blocks.block(COOLAIR);
    let ticket = w.t.open("plumbing", None).await;
    let party = COOLAIR.0.to_string();
    let pairs = [("party", party.as_str()), ("priority", "high")];
    w.t.assign(&w.coordinator, &ticket, "2026-11-03 09:00", &pairs)
        .await
        .assert_status(422);
    blocks.unblock(COOLAIR);
    w.t.assign(&w.coordinator, &ticket, "2026-11-03 09:00", &pairs)
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&ticket).await.assignee_party_id, Some(COOLAIR.0));
}

/// The module asks the guard as the system: the refusal is neutral.
#[renox::test]
async fn tkt08_the_refusal_is_neutral() {
    let (w, blocks) = world_with_blocks().await;
    blocks.block(COOLAIR);
    let message = vendor_block::refusal(w.t.state(), COOLAIR).await.unwrap();
    assert_eq!(message.as_deref(), Some("This party is on hold"));
    assert!(
        vendor_block::is_blocked(w.t.state(), COOLAIR)
            .await
            .unwrap()
    );
    let _: &Blocks = &blocks;
}
