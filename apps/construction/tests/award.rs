//! BL-CON-03 (#282): a blocked client's project keeps running and no new
//! project starts for it (`PTY.ACL.03`, block effect `continue` decided by the
//! MD). The quotation is wl-invoicing's and not built yet, so the step its
//! `accept` ends with (the project made from the award) is what is refused.
//! The set-off and the receipts are wl-payments' own transitions; their guard
//! (`money_out` waits for an exception approved by compliance, `money_in` is
//! never refused) is tested here at the contract the vertical relies on.

mod support;

use construction::ProjectStatus;
use renox::prelude::*;
use wl_attendance::service::{self, NewRecord};
use wl_kernel::PartyId;
use wl_kernel::block_effects::{BlockEffect, effect_for};
use wl_kernel::block_guard::{self, ActionKind, GuardOutcome};

use support::{boot, date, dubai};

const AWARD: &str = "/construction/projects/award";

/// The form of an award for `client`.
fn form(client: &str) -> Vec<(&str, &str)> {
    vec![
        ("client_party_id", client),
        ("code", "TWR-77"),
        ("name", "Palm Villas"),
        ("contract_value", "250000000"),
        ("award_document", "LPO-0042.pdf"),
    ]
}

/// Done when 1: given client C blocked, accepting the award is refused with
/// the neutral text and no project is made; once C is unblocked it starts.
#[renox::test]
async fn a_blocked_client_gets_no_new_project() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    t.blocked.block(PartyId(77));

    let refused = t.post(&pm, AWARD, &form("77")).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("This party is on hold"),
        "{}",
        refused.text()
    );
    let none = construction::Project::query()
        .where_(construction::Project::CODE.eq("TWR-77"))
        .first(t.db())
        .await
        .unwrap();
    assert!(none.is_none());

    // Another client is not affected, and C is accepted once unblocked.
    t.post(&pm, AWARD, &form("78")).await.assert_ok();
    t.blocked.unblock(PartyId(77));
    let mut again = form("77");
    again[1] = ("code", "TWR-78");
    t.post(&pm, AWARD, &again).await.assert_ok();
    let started = construction::Project::query()
        .where_(construction::Project::CODE.eq("TWR-78"))
        .first(t.db())
        .await
        .unwrap()
        .expect("the project");
    assert_eq!(started.status, ProjectStatus::Setup);
    assert_eq!(started.client_party_id, 77);
}

/// The award needs its document and the right to award.
#[renox::test]
async fn an_award_needs_its_document_and_the_permission() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let outsider = User::register(t.db(), "Omar", "omar@example.com", "password123")
        .await
        .unwrap();
    let mut no_document = form("80");
    no_document[4] = ("award_document", " ");
    t.post(&pm, AWARD, &no_document).await.assert_status(422);
    t.post(&outsider, AWARD, &form("80"))
        .await
        .assert_status(403);
}

/// Done when 2: C's `in_progress` project still accepts attendance; blocking
/// the client does not hold, terminate or otherwise change it. (IPA
/// submission is tested with the submit in `supersede.rs`; variations are not
/// built yet.)
#[renox::test]
async fn the_running_project_of_a_blocked_client_goes_on() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let site = t.work_location(&project).await;
    let worker = t.staff.employ(1);
    t.blocked.block(support::CLIENT_PARTY);

    t.on(date(2027, 3, 2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(
                worker,
                site.id,
                dubai(date(2027, 3, 2), 7, 0),
                Some(dubai(date(2027, 3, 2), 15, 0)),
                pm.id,
            ),
        )
        .await
        .unwrap();
        assert_eq!(record.date, date(2027, 3, 2));
    })
    .await;
    assert_eq!(t.reload(&project).await.status, ProjectStatus::InProgress);
    assert!(
        wl_kernel::approvals::ApprovalRequest::pending_about(t.db(), &project.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// The kind `project` continues, decided by the MD; nothing creates a review
/// task for it.
#[renox::test]
async fn the_block_effect_of_a_project_is_continue_decided_by_the_md() {
    let _t = boot().await;
    let rule = effect_for("project", &renox::auth::permissions::Scope::global());
    assert_eq!(rule.effect, BlockEffect::Continue);
    assert_eq!(
        rule.decider_role.as_deref(),
        Some(wl_kernel::COMPANY_ADMIN_ROLE)
    );
}

/// Done when 3 and 4: money out to the blocked client (the set-off of a
/// supplier bill from it) is refused by the guard and an exception is decided
/// by compliance; a receipt from it (`money_in`, a cheque) is never refused.
#[renox::test]
async fn money_out_to_a_blocked_client_waits_and_money_in_does_not() {
    let t = boot().await;
    t.blocked.block(support::CLIENT_PARTY);
    let guard = block_guard::guard(t.state());
    let mut tx = t.db().begin().await.unwrap();

    let out = guard
        .check_in(&mut tx, None, support::CLIENT_PARTY, ActionKind::MoneyOut)
        .await
        .unwrap();
    assert!(matches!(out, GuardOutcome::Blocked(_)));
    let income = guard
        .check_in(&mut tx, None, support::CLIENT_PARTY, ActionKind::MoneyIn)
        .await
        .unwrap();
    assert!(income.is_allowed());
    let start = guard
        .check_in(&mut tx, None, support::CLIENT_PARTY, ActionKind::Start)
        .await
        .unwrap();
    assert!(!start.is_allowed());
    drop(tx);
    assert_eq!(guard.exception_step().role, "compliance");
}
