//! BL-TKT-07 (#152): when a vendor's guarding document expires
//! (`DocumentExpired`), its contract carries the vendor document flag, its
//! open planned visits are flagged, the covered subjects get the compliance
//! flag `vendor_document_expired`, and the coordinator and the manager are
//! told; everything clears when a renewed document is accepted.

mod support;

use wl_kernel::PartyId;
use wl_kernel::documents::{
    BlockEntry, BlockMode, DocumentAccepted, DocumentExpired, DocumentId, OverrideMode,
};
use wl_kernel::tickets::{Assignee, ComplianceFlagReason};
use wl_ticketing::contract_end;
use wl_ticketing::flags::{ComplianceFlag, DOCUMENT_RENEWED, REPLACEMENT_ACTIVE};
use wl_ticketing::vendor_flags::NOTIFICATION_TYPE;
use wl_ticketing::{ContractStatus, ServiceContract, Ticket};

use support::contracts::{World, date, tower_a_cleaning, tower_a_fire_alarm, world_with_documents};
use support::documents::{CIVIL_DEFENCE, Documents};
use support::{BLUEFIRE, SAFEFIRE};

/// AMC-2026-0011: SafeFire's contract for the fire alarm with 2 open planned
/// visits, while its Civil Defence approval is still valid.
struct Fire {
    contract: ServiceContract,
    first: Ticket,
    second: Ticket,
}

async fn fire(w: &World, docs: &Documents) -> Fire {
    docs.set(&SAFEFIRE.subject(), CIVIL_DEFENCE, Some(date("2026-11-30")));
    let contract = w
        .contract(
            SAFEFIRE,
            "2026-01-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;
    let vendor = Some(Assignee::party(SAFEFIRE));
    let first = w
        .visit(&contract, "fire_safety", tower_a_fire_alarm(), "2026-12-10", vendor)
        .await;
    let second = w
        .visit(&contract, "fire_safety", tower_a_fire_alarm(), "2027-03-10", vendor)
        .await;
    Fire {
        contract,
        first,
        second,
    }
}

/// The expiry of `vendor`'s approval, as `wl-documents` announces it.
fn expired(docs: &Documents, vendor: PartyId) -> DocumentExpired {
    DocumentExpired {
        document: DocumentId(9),
        subject: vendor.subject(),
        kind: CIVIL_DEFENCE.into(),
        expires_on: date("2026-11-30"),
        blocks: docs.all_entries(),
        blocked: Vec::new(),
    }
}

/// A renewed approval, valid to `expires`, as `wl-documents` announces it.
fn accepted(vendor: PartyId, expires: &str) -> DocumentAccepted {
    DocumentAccepted {
        document: DocumentId(10),
        subject: vendor.subject(),
        kind: CIVIL_DEFENCE.into(),
        expires_on: Some(date(expires)),
    }
}

async fn open_flags(w: &World) -> Vec<ComplianceFlag> {
    ComplianceFlag::open(w.t.db()).await.unwrap()
}

/// AC1: when the approval expires, the contract shows the vendor document
/// flag, both visits are flagged, Tower A fire alarm has a
/// `vendor_document_expired` flag, and the coordinator and the property
/// manager are told, once.
#[renox::test]
async fn tkt07_an_expired_approval_flags_the_contract_its_visits_and_its_subject() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();

    let contract = w.reload(&f.contract).await;
    assert!(contract.vendor_document_flag);
    assert_eq!(contract.vendor_document_kind.as_deref(), Some(CIVIL_DEFENCE));
    assert!(w.t.reload(&f.first).await.vendor_document_flag);
    assert!(w.t.reload(&f.second).await.vendor_document_flag);

    let flags = open_flags(&w).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, ComplianceFlagReason::VendorDocumentExpired);
    assert_eq!(flags[0].subject(), tower_a_fire_alarm());
    assert_eq!(flags[0].contract_id, Some(f.contract.id));
    assert_eq!(w.raised.all().len(), 1);
    assert_eq!(w.raised.all()[0].reason, "vendor_document_expired");

    assert_eq!(w.told(NOTIFICATION_TYPE, &w.coordinator), 1);
    assert_eq!(w.told(NOTIFICATION_TYPE, &w.manager), 1);

    // The page shows the flag.
    w.t.app.acting_as(&w.coordinator);
    w.t.app
        .get(&format!("/service-contracts/{}", f.contract.id))
        .await
        .assert_ok()
        .assert_see("Vendor document expired");

    // A repeated event raises nothing twice.
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();
    assert_eq!(open_flags(&w).await.len(), 1);
    assert_eq!(w.raised.all().len(), 1);
    assert_eq!(w.told(NOTIFICATION_TYPE, &w.coordinator), 1);
}

/// The flag does not reach other vendors, other categories or other kinds
/// of document.
#[renox::test]
async fn tkt07_only_the_contracts_the_document_guards_are_flagged() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    // The same vendor's cleaning contract: the approval does not guard it.
    let cleaning = w
        .contract(
            SAFEFIRE,
            "2026-01-01",
            "2027-12-31",
            &[(tower_a_cleaning(), "cleaning")],
        )
        .await;
    // Another vendor's fire contract.
    docs.set(&BLUEFIRE.subject(), CIVIL_DEFENCE, Some(date("2027-12-31")));
    let other = w
        .contract(
            BLUEFIRE,
            "2026-01-01",
            "2027-12-31",
            &[(support::contracts::tower_a_lifts(), "lifts")],
        )
        .await;

    // A document kind that guards something else is no business of ours.
    let mut elsewhere = expired(&docs, SAFEFIRE);
    elsewhere.blocks = vec![BlockEntry {
        kind: CIVIL_DEFENCE.into(),
        guarded: "deployment.activate".into(),
        mode: BlockMode::Block,
        override_mode: OverrideMode::Never,
        condition: None,
        locked: true,
    }];
    w.t.state().emit(elsewhere).await.unwrap();
    assert!(!w.reload(&f.contract).await.vendor_document_flag);

    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();
    assert!(w.reload(&f.contract).await.vendor_document_flag);
    assert!(!w.reload(&cleaning).await.vendor_document_flag);
    assert!(!w.reload(&other).await.vendor_document_flag);
    assert_eq!(open_flags(&w).await.len(), 1);
}

/// A planned visit created while the contract carries the flag is flagged.
#[renox::test]
async fn tkt07_a_new_visit_of_a_flagged_contract_is_flagged() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();
    let contract = w.reload(&f.contract).await;
    let late = w
        .visit(
            &contract,
            "fire_safety",
            tower_a_fire_alarm(),
            "2027-06-10",
            Some(Assignee::party(SAFEFIRE)),
        )
        .await;
    assert!(late.vendor_document_flag);
}

/// AC2: when a renewed approval is accepted, the contract flag, the visit
/// flags and the compliance flag clear and `ComplianceFlagCleared` is emitted.
#[renox::test]
async fn tkt07_a_renewed_approval_clears_everything() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();

    // An old document accepted late, or another kind, clears nothing.
    let stale = accepted(SAFEFIRE, "2026-01-31");
    w.t.state().emit(stale).await.unwrap();
    let mut other_kind = accepted(SAFEFIRE, "2027-11-30");
    other_kind.kind = "trade_licence".into();
    w.t.state().emit(other_kind).await.unwrap();
    assert!(w.reload(&f.contract).await.vendor_document_flag);
    assert_eq!(open_flags(&w).await.len(), 1);

    w.t.state().emit(accepted(SAFEFIRE, "2027-11-30")).await.unwrap();
    let contract = w.reload(&f.contract).await;
    assert!(!contract.vendor_document_flag);
    assert_eq!(contract.vendor_document_kind, None);
    assert!(!w.t.reload(&f.first).await.vendor_document_flag);
    assert!(!w.t.reload(&f.second).await.vendor_document_flag);
    assert!(open_flags(&w).await.is_empty());
    let cleared = w.cleared.all();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].reason, "vendor_document_expired");
    assert_eq!(cleared[0].subject, tower_a_fire_alarm());
    let flag = ComplianceFlag::query().first(w.t.db()).await.unwrap().unwrap();
    assert_eq!(flag.cleared_by.as_deref(), Some(DOCUMENT_RENEWED));
}

/// AC3: reassigning a visit to "BlueFire" records `vendor_document_expired`
/// and removes that visit's flag; the Tower A compliance flag stays.
#[renox::test]
async fn tkt07_reassigning_a_visit_removes_its_flag_only() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();
    docs.set(&BLUEFIRE.subject(), CIVIL_DEFENCE, Some(date("2027-12-31")));

    let party = BLUEFIRE.0.to_string();
    w.t.assign(
        &w.coordinator,
        &f.first,
        "2026-12-01 09:00",
        &[("party", party.as_str()), ("reason_code", "vendor_document_expired")],
    )
    .await
    .assert_ok();
    let first = w.t.reload(&f.first).await;
    assert_eq!(first.assignee_party_id, Some(BLUEFIRE.0));
    assert!(!first.vendor_document_flag);
    let history = w.t.history(&first).await;
    assert_eq!(history.last().unwrap().reason_code.as_deref(), Some("vendor_document_expired"));

    assert!(w.t.reload(&f.second).await.vendor_document_flag);
    assert!(w.reload(&f.contract).await.vendor_document_flag);
    assert_eq!(open_flags(&w).await.len(), 1, "the subject stays flagged");
}

/// AC4: activating replacement contract AMC-2026-0020 with "BlueFire"
/// covering Tower A fire alarm clears the subject's flag, while the first
/// contract keeps its vendor document flag.
#[renox::test]
async fn tkt07_a_replacement_contract_clears_the_subjects_flag_only() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();
    assert_eq!(open_flags(&w).await.len(), 1);

    docs.set(&BLUEFIRE.subject(), CIVIL_DEFENCE, Some(date("2027-12-31")));
    let replacement = w
        .contract(
            BLUEFIRE,
            "2026-12-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;
    assert_eq!(replacement.status, ContractStatus::Active);
    assert!(open_flags(&w).await.is_empty());
    let flag = ComplianceFlag::query().first(w.t.db()).await.unwrap().unwrap();
    assert_eq!(flag.cleared_by.as_deref(), Some(REPLACEMENT_ACTIVE));
    assert_eq!(w.cleared.all().len(), 1);
    assert!(w.reload(&f.contract).await.vendor_document_flag, "its own flag stays");
}

/// While another flagged contract still covers the subject, a renewal of one
/// vendor's document does not clear the subject's flag.
#[renox::test]
async fn tkt07_a_subject_stays_flagged_while_another_flagged_contract_covers_it() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    docs.set(&BLUEFIRE.subject(), CIVIL_DEFENCE, Some(date("2026-11-30")));
    let second = w
        .contract(
            BLUEFIRE,
            "2026-01-01",
            "2027-12-31",
            &[(tower_a_fire_alarm(), "fire_safety")],
        )
        .await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();
    w.t.state().emit(expired(&docs, BLUEFIRE)).await.unwrap();
    assert_eq!(open_flags(&w).await.len(), 1, "one open flag per subject, category and reason");

    w.t.state().emit(accepted(SAFEFIRE, "2027-11-30")).await.unwrap();
    assert!(!w.reload(&f.contract).await.vendor_document_flag);
    assert!(w.reload(&second).await.vendor_document_flag);
    assert_eq!(open_flags(&w).await.len(), 1);

    w.t.state().emit(accepted(BLUEFIRE, "2027-11-30")).await.unwrap();
    assert!(open_flags(&w).await.is_empty());
}

/// AC5: when AMC-2026-0011 is terminated with no replacement, on the
/// termination date the `vendor_document_expired` flag clears and
/// `no_contract` is raised.
#[renox::test]
async fn tkt07_the_contract_end_replaces_the_flag_with_no_contract() {
    let (w, docs) = world_with_documents().await;
    let f = fire(&w, &docs).await;
    w.t.state().emit(expired(&docs, SAFEFIRE)).await.unwrap();

    w.terminate(&f.contract, "Approval lapsed", "2026-12-15").await.assert_ok();
    assert_eq!(open_flags(&w).await[0].reason, ComplianceFlagReason::VendorDocumentExpired);

    contract_end::run(w.t.state(), date("2026-12-15")).await.unwrap();
    let flags = open_flags(&w).await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].reason, ComplianceFlagReason::NoContract);
    assert_eq!(flags[0].subject(), tower_a_fire_alarm());
    let cleared = w.cleared.all();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].reason, "vendor_document_expired");
}
