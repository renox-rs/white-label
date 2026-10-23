//! BL-FNB-09 (#373), 06 §4.3 row *Wrong file imported, or a file imported
//! twice*, §7.1 *Payout statement* (`import`, `void`), `PLT.ACL.09`: a payout
//! statement with the same aggregator and reference as one that is not void is
//! refused and points to the existing one; a wrong file is voided before it is
//! settled (a matched or disputed one needs a second holder's approval), its
//! matching and receivable are reversed, its refund returns run `reverse` with
//! the reason *statement voided*, the number is kept and the reference can be
//! imported again; a settled statement cannot be voided. The modules are the
//! real wl-sales, wl-payments, wl-inventory and wl-delivery.

mod support;

use fnb::{PayoutLine, PayoutStatus};
use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::voids::VoidRecord;
use wl_sales::{ReturnKind, ReturnResolution, ReturnStatus, SalesReturn};

use support::payouts::{completed, file, import, reload, sales_of, staff, statement, void};
use support::{World, boot_own_riders};

/// The returns of the sales order of `order`.
async fn returns_of(w: &World, order: &fnb::FnbOrder) -> Vec<SalesReturn> {
    SalesReturn::query()
        .where_(SalesReturn::SALE_ID.eq(order.sales_order_id))
        .order_by(SalesReturn::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The statements stored.
async fn statements(w: &World) -> Vec<fnb::PayoutStatement> {
    fnb::PayoutStatement::query()
        .order_by(fnb::PayoutStatement::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// AC1: given Talabat statement "TB-2026-10-W2" imported, importing it again
/// (also " tb-2026-10-w2 ") is refused and points to the existing statement.
#[renox::test]
async fn importing_a_statement_again_is_refused_and_names_the_existing_one() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let content = file(&["TB-5001,60.00,12.00,0,0,48.00,"]);

    import(&w, &s.accountant, "TB-2026-10-W2", &content)
        .await
        .assert_status(303);
    let first = statement(&w, "TB-2026-10-W2").await;
    assert_eq!(first.status, PayoutStatus::Imported);
    assert_eq!(first.line_count, 1);

    for reference in ["TB-2026-10-W2", " tb-2026-10-w2 "] {
        let again = import(&w, &s.accountant, reference, &content).await;
        again.assert_status(422);
        assert!(
            again.text().contains(&first.number),
            "the refusal names {}: {}",
            first.number,
            again.text()
        );
        assert!(again.text().contains(&format!("/payouts/{}", first.id)));
    }
    assert_eq!(
        statements(&w).await.len(),
        1,
        "no second statement was made"
    );
}

/// Only a holder of `reconciliation.manage` imports; a reader sees the list.
#[renox::test]
async fn only_the_accountant_imports_and_a_reader_only_looks() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let content = file(&["TB-5001,60.00,12.00,0,0,48.00,"]);

    import(&w, &s.reader, "TB-1", &content)
        .await
        .assert_status(403);
    import(&w, &w.nobody, "TB-1", &content)
        .await
        .assert_status(403);
    assert!(statements(&w).await.is_empty());

    import(&w, &s.accountant, "TB-1", &content)
        .await
        .assert_status(303);
    w.get(&s.reader, "/payouts")
        .await
        .assert_ok()
        .assert_see("TB-1");
    w.get(&w.nobody, "/payouts").await.assert_status(403);
}

/// AC2: given that statement `matched` with receivable AED 12,400.00 and two
/// refund returns, `void` with reason "wrong week" needs approval; after
/// approval it is `voided`, the receivable is reversed, both returns are
/// `reversed` with reason *statement voided* and the two orders' amount
/// refunded go back to 0.
#[renox::test]
async fn voiding_a_matched_statement_needs_approval_and_reverses_the_receivable_and_the_returns() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let first = completed(&w, "TB-ORD-1001", "60.00").await;
    let second = completed(&w, "TB-ORD-1002", "40.00").await;
    let content = file(&[
        "TB-ORD-1001,60.00,12.00,0,25.00,23.00,Food was cold",
        "TB-ORD-1002,40.00,8.00,0,10.00,22.00,Missing item",
        "TB-FEES,0,0,0,0,12355.00,",
    ]);
    import(&w, &s.accountant, "TB-2026-10-W2", &content)
        .await
        .assert_status(303);
    let imported = statement(&w, "TB-2026-10-W2").await;
    // The platform fee line has no order: the accountant explains it.
    let lines = PayoutLine::of_statement(w.db(), imported.id).await.unwrap();
    let fees = lines.iter().find(|l| l.order_number == "TB-FEES").unwrap();
    assert!(fees.is_unmatched());
    w.post(
        &s.accountant,
        &format!("/payouts/{}/lines/{}/explain", imported.id, fees.id),
        &[("note", "Platform fee, no order")],
    )
    .await
    .assert_status(303);
    w.post(
        &s.accountant,
        &format!("/payouts/{}/match", imported.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    let matched = reload(&w, &imported).await;
    assert_eq!(matched.status, PayoutStatus::Matched);
    assert_eq!(matched.receivable_amount, 1_240_000, "AED 12,400.00");
    assert!(matched.receivable_booked_at.is_some());
    for (order, amount) in [(&first, 2_500), (&second, 1_000)] {
        let returns = returns_of(&w, order).await;
        assert_eq!(returns.len(), 1);
        assert_eq!(returns[0].kind, ReturnKind::NoGoods);
        assert_eq!(
            returns[0].resolution,
            Some(ReturnResolution::RefundedByChannel)
        );
        assert_eq!(returns[0].status, ReturnStatus::Resolved);
        assert_eq!(sales_of(&w, order).await.refunded_amount, amount);
    }

    // Void needs an approval: nothing changes yet.
    void(&w, &s.accountant, imported.id, "wrong week").await;
    assert_eq!(reload(&w, &imported).await.status, PayoutStatus::Matched);
    let requests = ApprovalRequest::pending_about(w.db(), &imported.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(sales_of(&w, &first).await.refunded_amount, 2_500);
    // The requester cannot approve; the second holder does.
    assert!(
        approvals::approve(w.state(), requests[0].id, &s.accountant, "")
            .await
            .is_err()
    );
    approvals::approve(w.state(), requests[0].id, &s.approver, "")
        .await
        .unwrap();

    let voided = reload(&w, &imported).await;
    assert_eq!(voided.status, PayoutStatus::Voided);
    assert_eq!(voided.receivable_amount, 0, "the receivable is reversed");
    assert!(voided.receivable_booked_at.is_none());
    assert_eq!(voided.void_reason.as_deref(), Some("wrong week"));
    for order in [&first, &second] {
        let returns = returns_of(&w, order).await;
        assert_eq!(returns[0].status, ReturnStatus::Reversed);
        assert_eq!(
            returns[0].closing_reason.as_deref(),
            Some("statement voided")
        );
        assert_eq!(sales_of(&w, order).await.refunded_amount, 0);
    }
    // The number is kept, never deleted and never given again.
    let record = VoidRecord::of_subject(w.db(), &imported.subject())
        .await
        .unwrap()
        .expect("a Void record");
    assert_eq!(record.number.as_deref(), Some(imported.number.as_str()));
    assert_eq!(statements(&w).await.len(), 1);
}

/// AC3: after the void, "TB-2026-10-W2" can be imported again, with its own
/// number.
#[renox::test]
async fn after_the_void_the_reference_can_be_imported_again_with_a_new_number() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    let content = file(&["TB-5001,60.00,12.00,0,0,48.00,"]);
    import(&w, &s.accountant, "TB-2026-10-W2", &content)
        .await
        .assert_status(303);
    let wrong = statement(&w, "TB-2026-10-W2").await;

    // An imported statement has no receivable booked: no approval is asked.
    void(&w, &s.accountant, wrong.id, "wrong week").await;
    let voided = reload(&w, &wrong).await;
    assert_eq!(voided.status, PayoutStatus::Voided);
    assert!(
        ApprovalRequest::pending_about(w.db(), &wrong.subject())
            .await
            .unwrap()
            .is_empty()
    );

    import(&w, &s.accountant, " TB-2026-10-W2", &content)
        .await
        .assert_status(303);
    let right = statement(&w, "TB-2026-10-W2").await;
    assert_ne!(right.id, wrong.id);
    assert_ne!(right.number, wrong.number);
    assert_eq!(right.status, PayoutStatus::Imported);
    assert_eq!(statements(&w).await.len(), 2, "the voided one is kept");
    // And the new one is a duplicate again.
    import(&w, &s.accountant, "TB-2026-10-W2", &content)
        .await
        .assert_status(422);
}

/// A void needs a reason.
#[renox::test]
async fn a_void_without_a_reason_is_refused() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    import(
        &w,
        &s.accountant,
        "TB-9",
        &file(&["TB-5001,60.00,12.00,0,0,48.00,"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "TB-9").await;
    void(&w, &s.accountant, st.id, "   ")
        .await
        .assert_status(422);
    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Imported);
}

/// AC4: `void` on a `settled` statement is refused.
#[renox::test]
async fn a_settled_statement_cannot_be_voided() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    completed(&w, "TB-ORD-2001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "TB-2026-10-W3",
        &file(&["TB-ORD-2001,60.00,12.00,0,0,48.00,"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "TB-2026-10-W3").await;
    w.post(
        &s.accountant,
        &format!("/payouts/{}/match", st.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    fnb::payouts::settle(w.state(), st.id).await.unwrap();
    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Settled);

    void(&w, &s.accountant, st.id, "wrong week")
        .await
        .assert_status(422);
    let after = reload(&w, &st).await;
    assert_eq!(after.status, PayoutStatus::Settled);
    assert_eq!(after.receivable_amount, 4_800);
    assert!(
        ApprovalRequest::pending_about(w.db(), &st.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// The statement's page offers Void to a holder of the permission on a
/// statement that can still be voided, and says why not on a settled one.
#[renox::test]
async fn the_page_offers_void_until_the_statement_is_settled() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    completed(&w, "TB-ORD-3001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "TB-2026-10-W4",
        &file(&["TB-ORD-3001,60.00,12.00,0,0,48.00,"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "TB-2026-10-W4").await;
    let path = format!("/payouts/{}", st.id);

    w.get(&s.accountant, &path)
        .await
        .assert_ok()
        .assert_see("Void")
        .assert_see("Match");
    // A reader sees the statement but none of the buttons.
    w.get(&s.reader, &path)
        .await
        .assert_ok()
        .assert_dont_see("Void")
        .assert_dont_see("Dispute");

    w.post(&s.accountant, &format!("{path}/match"), &[("x", "")])
        .await
        .assert_status(303);
    fnb::payouts::settle(w.state(), st.id).await.unwrap();
    w.get(&s.accountant, &path)
        .await
        .assert_ok()
        .assert_see("cannot be voided");
}

/// AC2 (disputed): voiding a statement that is `disputed` closes its open
/// follow-up task, and the statement is `voided` once the second holder
/// approves.
#[renox::test]
async fn voiding_a_disputed_statement_closes_the_dispute_task() {
    let w = boot_own_riders().await;
    let s = staff(&w).await;
    completed(&w, "ORD-1001", "60.00").await;
    import(
        &w,
        &s.accountant,
        "AGG-W43",
        &file(&["ORD-1001,60.00,12.00,0,25.00,23.00,Food was cold"]),
    )
    .await
    .assert_status(303);
    let st = statement(&w, "AGG-W43").await;
    w.post(
        &s.accountant,
        &format!("/payouts/{}/match", st.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let lines = PayoutLine::of_statement(w.db(), st.id).await.unwrap();
    let field = format!("line_{}", lines[0].id);
    w.post(
        &s.accountant,
        &format!("/payouts/{}/dispute", st.id),
        &[("reason", "Commission is wrong"), (field.as_str(), "on")],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Disputed);
    let key = format!("fnb.payout_dispute:{}", st.id);
    assert_eq!(w.tickets.opened_with(&key).len(), 1, "a follow-up task");
    assert!(w.tickets.closed().is_empty());

    void(&w, &s.accountant, st.id, "wrong week").await;
    let requests = ApprovalRequest::pending_about(w.db(), &st.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    approvals::approve(w.state(), requests[0].id, &s.approver, "")
        .await
        .unwrap();

    assert_eq!(reload(&w, &st).await.status, PayoutStatus::Voided);
    assert_eq!(w.tickets.closed().len(), 1, "the dispute task is closed");
}
