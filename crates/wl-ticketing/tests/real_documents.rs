//! BL-TKT-06 and BL-TKT-07 (#151, #152) through the real `wl-documents`
//! instead of the test double: the Civil Defence approval is a real kind
//! (01's shape: a legal block, override mode `never`, `blocks` on
//! `ticket.resolve` and `service_contract.activate` for fire and life
//! safety), the vendor's approval a real document that a reviewer accepts,
//! and the guard is `wl-documents`' own.

mod support;

use renox::prelude::*;
use wl_documents::{Documents, KindSeed, NewDocument, WlDocuments, permissions as doc_perm};
use wl_kernel::documents::{BlockCondition, BlockMode, OverrideMode};
use wl_kernel::tickets::Assignee;
use wl_ticketing::{ContractStatus, Ticket, TicketStatus, WlTicketing};

use support::SAFEFIRE;
use support::contracts::{World, date, tower_a_fire_alarm, world_customized};

const KIND: &str = "civil_defence_approval";

/// 01's Civil Defence approval kind.
fn civil_defence() -> KindSeed {
    let fire = BlockCondition::new("category", ["fire_safety"]);
    KindSeed::new(KIND, "Civil Defence approval")
        .subject_type(wl_kernel::parties::PARTY_SUBJECT_TYPE)
        .expires()
        .numbered()
        .responsible_role("coordinator")
        .legal_block()
        .blocks_when("service_contract.activate", BlockMode::Block, fire.clone())
        .blocks_when("ticket.resolve", BlockMode::Block, fire)
}

async fn real_world() -> World {
    let world = world_customized(
        WlTicketing::new().compliance_manager_role("property_manager"),
        |app| app.module(WlDocuments),
    )
    .await;
    wl_documents::kinds::install_seeds(world.t.db(), &[civil_defence()])
        .await
        .expect("the kind installs");
    world
}

/// SafeFire's approval, valid through `expires`, accepted by a reviewer.
async fn approval(w: &World, expires: &str) {
    let email = "rita@example.com";
    let reviewer = match User::find_by_email(w.t.db(), email).await.unwrap() {
        Some(user) => user,
        None => w.t.user("Rita", &[doc_perm::VIEW, doc_perm::REVIEW]).await,
    };
    let mut new = NewDocument::new(
        SAFEFIRE.subject(),
        KIND,
        Upload::new(
            "approval.pdf",
            "application/pdf",
            b"%PDF-1.7 approval".to_vec(),
        ),
    )
    .expires_on(date(expires));
    new.number = Some("CD-2026-114".into());
    let document = Documents::attach(w.t.state(), None, new)
        .await
        .expect("attached")
        .expect("the document is attached");
    w.t.app.acting_as(&reviewer);
    w.t.app
        .htmx()
        .post(&format!("/documents/{}/accept", document.id), &[])
        .await
        .assert_ok();
}

async fn started_visit(w: &World) -> Ticket {
    // The approval counts from the day it was accepted, so the contract
    // starts after that.
    let contract = w
        .contract(
            SAFEFIRE,
            "2026-11-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;
    assert_eq!(contract.status, ContractStatus::Active);
    let visit = w
        .visit(
            &contract,
            "fire_safety",
            tower_a_fire_alarm(),
            "2026-12-01",
            Some(Assignee::party(SAFEFIRE)),
        )
        .await;
    w.start(&visit).await
}

/// #151 AC1 and AC4 with the real guard: the approval expired on 11-30, the
/// resolve of the fire-alarm visit on 12-02 is refused naming the document
/// and the date, with no override offered; on 11-30 it resolves.
#[renox::test]
async fn tkt06_the_real_guard_refuses_the_resolve_while_the_approval_is_expired() {
    let w = real_world().await;
    approval(&w, "2026-11-30").await;
    let visit = started_visit(&w).await;

    w.t.run(&w.coordinator, &visit, "resolve", "2026-12-02 10:00", &[])
        .await
        .assert_status(422)
        .assert_see("Civil Defence approval expired on 2026-11-30")
        .assert_see("override: not allowed")
        .assert_dont_see("overrides/new");
    assert_eq!(w.t.reload(&visit).await.status, TicketStatus::InProgress);

    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .htmx()
        .post(
            &format!("/tickets/{}/service-report", visit.id),
            &[("reference", "SR-2026-114")],
        )
        .await
        .assert_ok();
    w.t.run(&w.coordinator, &visit, "resolve", "2026-11-30 10:00", &[])
        .await
        .assert_ok();
    assert_eq!(w.t.reload(&visit).await.status, TicketStatus::Resolved);
}

/// #151 AC3 with the real guard: a contract starting 2027-01-01 is refused
/// while the approval expires 2026-12-31, and allowed once a renewal valid to
/// 2027-12-31 is accepted. The kind is a legal block with override mode
/// `never`.
#[renox::test]
async fn tkt06_the_real_guard_needs_the_approval_on_the_start_date() {
    let w = real_world().await;
    let kind = wl_documents::DocumentKind::by_key(w.t.db(), KIND)
        .await
        .unwrap()
        .expect("the kind");
    assert_eq!(kind.override_mode, OverrideMode::Never);
    approval(&w, "2026-12-31").await;
    let draft = w
        .draft(
            SAFEFIRE,
            "2027-01-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;

    w.try_activate(&draft)
        .await
        .assert_status(422)
        .assert_see("Civil Defence approval expired on 2026-12-31")
        .assert_dont_see("overrides/new");
    assert_eq!(w.reload(&draft).await.status, ContractStatus::Draft);

    approval(&w, "2027-12-31").await;
    let active = w.activate(&draft).await;
    assert_eq!(active.status, ContractStatus::Active);
}

/// #152 AC1 and AC2 with the real documents module: the nightly expiry run
/// finds the approval expired and announces it; the contract, its visit and
/// Tower A are flagged and the coordinator and manager told; accepting a
/// renewed approval clears all of it.
#[renox::test]
async fn tkt07_the_real_expiry_run_flags_and_the_real_renewal_clears() {
    use wl_ticketing::flags::ComplianceFlag;
    use wl_ticketing::vendor_flags::NOTIFICATION_TYPE;

    let w = real_world().await;
    approval(&w, "2026-11-30").await;
    let contract = w
        .contract(
            SAFEFIRE,
            "2026-11-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;
    let visit = w
        .visit(
            &contract,
            "fire_safety",
            tower_a_fire_alarm(),
            "2026-12-10",
            Some(Assignee::party(SAFEFIRE)),
        )
        .await;

    // The night before the expiry date: nothing.
    let report = wl_documents::expiry::run::run(w.t.state(), date("2026-11-30"))
        .await
        .unwrap();
    assert_eq!(report.expired, 0);
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());

    let report = wl_documents::expiry::run::run(w.t.state(), date("2026-12-01"))
        .await
        .unwrap();
    assert_eq!(report.expired, 1);
    assert!(w.reload(&contract).await.vendor_document_flag);
    assert!(w.t.reload(&visit).await.vendor_document_flag);
    let flags = ComplianceFlag::open(w.t.db()).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].subject(), tower_a_fire_alarm());
    assert_eq!(w.told(NOTIFICATION_TYPE, &w.coordinator), 1);
    assert_eq!(w.told(NOTIFICATION_TYPE, &w.manager), 1);

    approval(&w, "2027-12-31").await;
    assert!(!w.reload(&contract).await.vendor_document_flag);
    assert!(!w.t.reload(&visit).await.vendor_document_flag);
    assert!(ComplianceFlag::open(w.t.db()).await.unwrap().is_empty());
    assert_eq!(w.cleared.all().len(), 1);
}
