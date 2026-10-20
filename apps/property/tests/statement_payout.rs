//! BL-PRP-20 (#270, `PRP.ACL.21`): a published statement is settled one of
//! three ways. A positive balance is paid out, but only when the statement is
//! not disputed, the owner is not blocked and a second user approved the
//! payout. A balance of zero or less is carried forward. When the agreement
//! allows it, or the balance stayed negative for three statements, the
//! property manager may invoice the negative balance to the owner.

mod support;

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::charge_type;
use property::statement::{self, OwnerStatement, StatementStatus};
use property::statement_machine::{INVOICE_ACTION, PAYOUT_ACTION};
use property::statement_payout::RECHARGED_COSTS;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::parties::PartyKind;
use wl_kernel::payments::{Direction, PaymentMethod};
use wl_kernel::{BLOCK_EXCEPTION_ACTION, PartyId, VatCategory};

use support::{ACCOUNTANT, Booted, MANAGER, aed, boot, day};

/// The owner.
const OWNER: PartyId = PartyId(31);

struct Scene {
    t: Booted,
    agreement: ManagementAgreement,
    accountant: User,
    manager: User,
}

/// An active agreement, an accountant (A) and a property manager (B).
async fn scene() -> Scene {
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
    Scene {
        t,
        agreement,
        accountant,
        manager,
    }
}

impl Scene {
    /// `user` runs the transition `name` on the statement `id`.
    async fn run(&self, user: &User, id: i64, name: &str) -> renox::testing::TestResponse {
        self.t
            .post_plain(
                user,
                &format!("/property/statements/{id}/transitions/{name}"),
                &[],
            )
            .await
    }

    /// A statement for a month of the past, published: the accountant adds one
    /// line of `kind` and `amount`, the manager approves and the accountant
    /// publishes.
    async fn published(&self, kind: &str, amount: &str, start: i64) -> OwnerStatement {
        let draft = statement::generate(
            self.t.state(),
            self.agreement.id,
            day(start),
            day(start + 29),
        )
        .await
        .unwrap()
        .expect("a draft");
        if !amount.is_empty() {
            self.t
                .post(
                    &self.accountant,
                    &format!("/property/statements/{}/lines", draft.id),
                    &[("kind", kind), ("description", "Line"), ("amount", amount)],
                )
                .await
                .assert_ok();
        }
        self.run(&self.manager, draft.id, "approve")
            .await
            .assert_status(303);
        self.run(&self.accountant, draft.id, "publish")
            .await
            .assert_status(303);
        self.statement(draft.id).await
    }

    async fn statement(&self, id: i64) -> OwnerStatement {
        OwnerStatement::find_or_404(self.t.db(), id).await.unwrap()
    }

    /// The accountant asks to pay out the statement.
    async fn pay_out(&self, id: i64, extra: &[(&str, &str)]) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.accountant,
                &format!("/property/statements/{id}/pay-out"),
                extra,
            )
            .await
    }

    /// The pending approval requests about the statement.
    async fn requests(&self, statement: &OwnerStatement) -> Vec<ApprovalRequest> {
        ApprovalRequest::pending_about(self.t.db(), &statement.subject())
            .await
            .unwrap()
    }
}

/// AC1: a published statement with a balance of AED 42,000.00, not disputed,
/// the owner not blocked, prepared by accountant A: `pay_out` creates an
/// approval request; A cannot approve; property manager B approves; a payout
/// of AED 42,000.00 is recorded and the statement is `settled`.
#[renox::test]
async fn bl_prp_20_a_positive_balance_is_paid_out_after_a_second_user_approves() {
    let s = scene().await;
    let published = s.published("collected", "42,000", -60).await;
    assert_eq!(published.balance(), aed(42_000));
    assert_eq!(published.prepared_by, Some(s.accountant.id));

    s.pay_out(published.id, &[]).await.assert_ok();
    let waiting = s.statement(published.id).await;
    assert_eq!(waiting.status, StatementStatus::Published);
    assert!(s.t.refunds.payments().is_empty());
    let requests = s.requests(&published).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, PAYOUT_ACTION);
    assert_eq!(requests[0].requester_id, s.accountant.id);

    // A cannot approve; B can.
    let refused = approve(s.t.state(), requests[0].id, &s.accountant, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    approve(s.t.state(), requests[0].id, &s.manager, "ok")
        .await
        .unwrap();

    let settled = s.statement(published.id).await;
    assert_eq!(settled.status, StatementStatus::Settled);
    assert_eq!(settled.payout_amount, Some(aed(42_000)));
    assert_eq!(settled.settled_how.as_deref(), Some("payout"));
    assert_eq!(settled.carried_forward, 0);
    let paid = s.t.refunds.payments();
    assert_eq!(paid.len(), 1);
    let (payment, by) = &paid[0];
    assert_eq!(payment.direction, Direction::Out);
    assert_eq!(payment.party, OWNER);
    assert_eq!(payment.amount, aed(42_000));
    assert_eq!(payment.method, PaymentMethod::Transfer);
    assert_eq!(
        payment.reference.as_deref(),
        Some(published.number().as_str())
    );
    assert_eq!(*by, Some(s.accountant.id));
    assert_eq!(settled.payout_payment_id, Some(1));

    // It is paid once, and a statement with a payout is not revised.
    s.run(&s.accountant, published.id, "revise")
        .await
        .assert_status(422);
    s.pay_out(published.id, &[]).await.assert_status(422);
    assert_eq!(s.t.refunds.payments().len(), 1);
}

/// AC2: with the dispute flag set, `pay_out` is refused; clearing the dispute
/// with an adjustment line allows it.
#[renox::test]
async fn bl_prp_20_a_disputed_statement_is_not_paid_out() {
    let s = scene().await;
    let published = s.published("collected", "10,000", -60).await;
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/disputes", published.id),
        &[("line", ""), ("comment", "The owner doubts the rent")],
    )
    .await
    .assert_ok();
    assert!(s.statement(published.id).await.disputed);

    let refused = s.pay_out(published.id, &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("disputed"), "{}", refused.text());
    assert!(s.requests(&published).await.is_empty());

    let dispute =
        property::statement_dispute::StatementDispute::of_statement(s.t.db(), published.id)
            .await
            .unwrap()[0]
            .id
            .to_string();
    s.t.post(
        &s.accountant,
        &format!("/property/statements/{}/disputes/clear", published.id),
        &[
            ("dispute", &dispute),
            ("outcome", "adjustment"),
            ("amount", "500"),
            ("direction", "debit"),
        ],
    )
    .await
    .assert_ok();
    s.pay_out(published.id, &[]).await.assert_ok();
    assert_eq!(s.requests(&published).await.len(), 1);
}

/// AC3: with the owner blocked, `pay_out` is refused with the neutral text; the
/// accountant may ask for an exception instead, which waits for the compliance
/// role and makes no payout.
#[renox::test]
async fn bl_prp_20_a_blocked_owner_is_refused_with_the_neutral_text() {
    let s = scene().await;
    let published = s.published("collected", "10,000", -60).await;
    s.t.blocks.block(OWNER);

    let refused = s.pay_out(published.id, &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("on hold"), "{}", refused.text());
    assert!(s.requests(&published).await.is_empty());
    assert!(s.t.refunds.payments().is_empty());

    s.pay_out(published.id, &[("block_exception", "1")])
        .await
        .assert_ok();
    let requests = s.requests(&published).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(requests[0].transition, "pay_out");
    assert_eq!(
        s.statement(published.id).await.status,
        StatementStatus::Published
    );
    assert!(s.t.refunds.payments().is_empty());
}

/// Only a positive balance is paid out: zero and negative balances are refused.
#[renox::test]
async fn bl_prp_20_a_balance_that_is_not_positive_is_not_paid_out() {
    let s = scene().await;
    let nothing = s.published("collected", "", -90).await;
    assert_eq!(nothing.balance(), 0);
    let refused = s.pay_out(nothing.id, &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("positive balance"),
        "{}",
        refused.text()
    );

    let negative = s.published("expense", "3,500", -60).await;
    assert_eq!(negative.balance(), -aed(3_500));
    s.pay_out(negative.id, &[]).await.assert_status(422);
    assert!(s.t.refunds.payments().is_empty());
}

/// AC4: a balance of AED -3,500.00 is carried forward: the next draft statement
/// shows brought forward -3,500.00.
#[renox::test]
async fn bl_prp_20_a_negative_balance_is_carried_forward() {
    let s = scene().await;
    let negative = s.published("expense", "3,500", -90).await;
    assert_eq!(negative.balance(), -aed(3_500));

    // The manager does not carry forward; the accountant does.
    s.run(&s.manager, negative.id, "carry_forward")
        .await
        .assert_forbidden();
    s.run(&s.accountant, negative.id, "carry_forward")
        .await
        .assert_status(303);
    let settled = s.statement(negative.id).await;
    assert_eq!(settled.status, StatementStatus::Settled);
    assert_eq!(settled.carried_forward, -aed(3_500));
    assert_eq!(settled.settled_how.as_deref(), Some("carried_forward"));
    assert!(s.t.refunds.payments().is_empty());

    let next = statement::generate(s.t.state(), s.agreement.id, day(-60), day(-31))
        .await
        .unwrap()
        .expect("a draft");
    assert_eq!(next.brought_forward, -aed(3_500));
}

/// A balance of exactly 0 is carried forward too, with nothing brought
/// forward; a positive balance is not carried forward.
#[renox::test]
async fn bl_prp_20_a_balance_of_zero_is_carried_forward_and_a_positive_one_is_not() {
    let s = scene().await;
    let zero = s.published("collected", "", -90).await;
    assert_eq!(zero.balance(), 0);
    s.run(&s.accountant, zero.id, "carry_forward")
        .await
        .assert_status(303);
    let settled = s.statement(zero.id).await;
    assert_eq!(settled.status, StatementStatus::Settled);
    assert_eq!(settled.carried_forward, 0);
    let next = statement::generate(s.t.state(), s.agreement.id, day(-60), day(-31))
        .await
        .unwrap()
        .expect("a draft");
    assert_eq!(next.brought_forward, 0);

    let positive = s.published("collected", "100", -30).await;
    s.run(&s.accountant, positive.id, "carry_forward")
        .await
        .assert_status(422);
    assert_eq!(
        s.statement(positive.id).await.status,
        StatementStatus::Published
    );
}

/// Three statements in a row, or the agreement's clause, allow the invoice.
async fn negative_history(s: &Scene, balances: [i64; 3]) -> Vec<OwnerStatement> {
    let mut found = Vec::new();
    for (index, balance) in balances.into_iter().enumerate() {
        let offset = i64::try_from(index).unwrap_or(0);
        let last = index == 2;
        let status = if last {
            StatementStatus::Published
        } else {
            StatementStatus::Settled
        };
        let mut row = OwnerStatement {
            agreement_id: s.agreement.id,
            owner_party_id: OWNER.0,
            period_start: day(-90 + offset * 30),
            period_end: day(-61 + offset * 30),
            revision: 1,
            collected: balance,
            carried_forward: if last { 0 } else { balance },
            status,
            ..Default::default()
        };
        let mut tx = s.t.db().begin().await.unwrap();
        row.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        found.push(row);
    }
    found
}

/// AC5: with balances -1,000, -2,000 and -3,500 on 3 consecutive statements,
/// `invoice_owner` invoices AED 3,500.00 to the owner from the management
/// company's entity once another property manager approves; with only 2
/// negative statements and the agreement flag off, it is refused.
#[renox::test]
async fn bl_prp_20_three_negative_statements_let_the_owner_be_invoiced() {
    let s = scene().await;
    let history = negative_history(&s, [-aed(1_000), -aed(2_000), -aed(3_500)]).await;
    let last = &history[2];
    let path = format!("/property/statements/{}/invoice-owner", last.id);

    // The VAT treatment of recharged costs is the client's to configure.
    s.t.post(&s.manager, &path, &[]).await.assert_status(422);
    charge_type::configure(
        s.t.db(),
        RECHARGED_COSTS,
        "Recharged costs",
        VatCategory::Exempt,
    )
    .await
    .unwrap();

    // The accountant cannot ask; a property manager does.
    s.t.post(&s.accountant, &path, &[]).await.assert_forbidden();
    s.t.post(&s.manager, &path, &[]).await.assert_ok();
    let requests = s.requests(last).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, INVOICE_ACTION);
    assert!(s.t.invoices.all().is_empty());
    // The requester cannot approve; another property manager does.
    let refused = approve(s.t.state(), requests[0].id, &s.manager, "").await;
    assert!(refused.is_err());
    let second = s.t.user_with("Maher", MANAGER).await;
    approve(s.t.state(), requests[0].id, &second, "ok")
        .await
        .unwrap();

    let settled = s.statement(last.id).await;
    assert_eq!(settled.status, StatementStatus::Settled);
    assert_eq!(settled.settled_how.as_deref(), Some("invoiced"));
    assert_eq!(settled.carried_forward, 0);
    let invoices = s.t.invoices.all();
    assert_eq!(invoices.len(), 1);
    let draft = &invoices[0].draft;
    assert_eq!(draft.party, OWNER);
    assert_eq!(draft.source, last.subject());
    assert_eq!(draft.lines.len(), 1);
    assert_eq!(draft.lines[0].unit_price, aed(3_500));
    assert_eq!(draft.lines[0].charge_type.as_deref(), Some(RECHARGED_COSTS));
    // From the management company's entity: the default one.
    assert!(draft.legal_entity_id.is_none());
    assert_eq!(settled.owner_invoice_id, Some(invoices[0].summary.id.0));
}

/// With only two negative statements and the flag off, `invoice_owner` is
/// refused; with the agreement's clause it is allowed whatever the history.
#[renox::test]
async fn bl_prp_20_two_negative_statements_do_not_allow_the_invoice() {
    let s = scene().await;
    charge_type::configure(
        s.t.db(),
        RECHARGED_COSTS,
        "Recharged costs",
        VatCategory::Exempt,
    )
    .await
    .unwrap();
    let history = negative_history(&s, [aed(500), -aed(2_000), -aed(3_500)]).await;
    let last = &history[2];
    let path = format!("/property/statements/{}/invoice-owner", last.id);

    let refused = s.t.post(&s.manager, &path, &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("does not allow"),
        "{}",
        refused.text()
    );
    assert!(s.requests(last).await.is_empty());

    ManagementAgreement::query()
        .where_(ManagementAgreement::ID.eq(s.agreement.id))
        .update(s.t.db(), &[("may_invoice_negative_balance", &true)])
        .await
        .unwrap();
    s.t.post(&s.manager, &path, &[]).await.assert_ok();
    assert_eq!(s.requests(last).await.len(), 1);

    // A positive or empty balance is never invoiced.
    let scene2 = scene().await;
    charge_type::configure(
        scene2.t.db(),
        RECHARGED_COSTS,
        "Recharged costs",
        VatCategory::Exempt,
    )
    .await
    .unwrap();
    ManagementAgreement::query()
        .where_(ManagementAgreement::ID.eq(scene2.agreement.id))
        .update(scene2.t.db(), &[("may_invoice_negative_balance", &true)])
        .await
        .unwrap();
    let positive = scene2.published("collected", "100", -60).await;
    scene2
        .t
        .post(
            &scene2.manager,
            &format!("/property/statements/{}/invoice-owner", positive.id),
            &[],
        )
        .await
        .assert_status(422);
}
