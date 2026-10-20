//! BL-PRP-20 (#270), the edge case "payout approval pending and the owner gets
//! blocked or the statement disputed": the final approval checks the guards
//! again and the payout does not happen.
//!
//! This is the only test of its binary on purpose: the kernel keeps the guard
//! of the final re-check in one slot of the process, which every booted app
//! overwrites, so a test that blocks a party and relies on the re-check cannot
//! share its process with another test that boots an app in parallel. The two
//! cases run one after the other.

mod support;

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::statement::{self, OwnerStatement, StatementStatus};
use property::statement_machine::PAYOUT_ACTION;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve};
use wl_kernel::parties::PartyKind;

use support::{ACCOUNTANT, Booted, MANAGER, aed, boot, day};

const OWNER: PartyId = PartyId(31);

struct Scene {
    t: Booted,
    accountant: User,
    manager: User,
    published: OwnerStatement,
}

/// A published statement of AED 42,000.00 whose payout the accountant asked
/// for; it waits for a second user.
async fn waiting() -> (Scene, ApprovalRequest) {
    let t = boot().await;
    t.parties
        .add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let manager = t.user_with("Mona", MANAGER).await;
    let draft = statement::generate(t.state(), agreement.id, day(-60), day(-31))
        .await
        .unwrap()
        .expect("a draft");
    t.post(
        &accountant,
        &format!("/property/statements/{}/lines", draft.id),
        &[
            ("kind", "collected"),
            ("description", "Line"),
            ("amount", "42,000"),
        ],
    )
    .await
    .assert_ok();
    for (user, name) in [(&manager, "approve"), (&accountant, "publish")] {
        t.post_plain(
            user,
            &format!("/property/statements/{}/transitions/{name}", draft.id),
            &[],
        )
        .await
        .assert_status(303);
    }
    let published = OwnerStatement::find_or_404(t.db(), draft.id).await.unwrap();
    assert_eq!(published.balance(), aed(42_000));
    t.post(
        &accountant,
        &format!("/property/statements/{}/pay-out", published.id),
        &[],
    )
    .await
    .assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &published.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(request.action, PAYOUT_ACTION);
    (
        Scene {
            t,
            accountant,
            manager,
            published,
        },
        request,
    )
}

impl Scene {
    /// What the approval of `request` by the manager leaves: the statement is
    /// still published, no payout was recorded and the approved request failed.
    async fn assert_no_payout(&self, request: &ApprovalRequest) {
        let after = OwnerStatement::find_or_404(self.t.db(), self.published.id)
            .await
            .unwrap();
        assert_eq!(after.status, StatementStatus::Published);
        assert_eq!(after.payout_payment_id, None);
        assert!(self.t.refunds.payments().is_empty());
        let request = ApprovalRequest::find(self.t.db(), request.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.status, ApprovalStatus::Failed);
    }
}

#[renox::test]
async fn bl_prp_20_the_final_approval_checks_the_guards_again() {
    // The owner is blocked after the request: the final approval re-checks the
    // party and the payout does not run.
    let (s, request) = waiting().await;
    s.t.blocks.block(OWNER);
    let decided = approve(s.t.state(), request.id, &s.manager, "ok").await;
    assert!(decided.is_ok(), "{decided:?}");
    s.assert_no_payout(&request).await;
    assert!(
        s.t.blocks
            .asked()
            .iter()
            .any(|(party, _)| *party == OWNER.0),
        "the guard was asked about the owner"
    );
    drop(s.accountant);

    // The statement is disputed after the request: the guards of `pay_out`
    // refuse the approved run.
    let (s, request) = waiting().await;
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/disputes", s.published.id),
        &[("line", ""), ("comment", "The owner doubts the rent")],
    )
    .await
    .assert_ok();
    let decided = approve(s.t.state(), request.id, &s.manager, "ok").await;
    assert!(decided.is_ok(), "{decided:?}");
    s.assert_no_payout(&request).await;
}
