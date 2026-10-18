//! The back office of a reopened invoice (#70 part 5): the invoice page shows
//! the removed allocation with its reason and the "Reopened" table, the
//! reversal task list needs `receivables.view`, and "Mark done" closes a
//! task.

use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::{
    Allocation, Invoice, InvoicingTask, TaskKind, TaskStatus, Unallocation, UnallocationOutcome,
};
use wl_kernel::Subject;
use wl_kernel::invoicing::{InvoiceStatus, ReopenReason};
use wl_parties::Party;

use host::roles::{self, ACCOUNTANT, MEMBER, RECEIVABLES};

/// A verified user with `role`.
async fn person(app: &TestApp, email: &str, role: &str) -> User {
    let mut user = User::register(app.db(), "Rita", email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

#[renox::test]
async fn the_invoice_page_and_the_task_list_show_a_reopened_invoice() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();

    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let mut invoice = Invoice {
        party_id: party.id,
        currency: "AED".into(),
        number: Some("INV-REOPEN-0001".into()),
        gross_total: 200_000,
        balance: 200_000,
        status: InvoiceStatus::Issued,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    invoice
        .insert_in_state(&mut tx, InvoiceStatus::Issued, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let payment = Subject::new("payments.payment", 7);
    let mut allocation = Allocation::new(invoice.id, &payment, 200_000, None);
    allocation.removed_at = Some(renox::db::now());
    allocation.removed_reason = Some("payment_reversed".into());
    allocation.removed_key = Some("payments.allocation:1".into());
    allocation.insert(app.db()).await.unwrap();
    let mut log = Unallocation {
        invoice_id: invoice.id,
        source_type: payment.subject_type.clone(),
        source_id: payment.subject_id,
        amount: 200_000,
        key: "payments.allocation:1".into(),
        reason: ReopenReason::PaymentReversed,
        reversal_reason: Some("bank_return".into()),
        outcome: UnallocationOutcome::Done,
        transition: Some("unallocate".into()),
        balance_after: Some(200_000),
        ..Default::default()
    };
    log.insert(app.db()).await.unwrap();
    let mut task = InvoicingTask {
        kind: TaskKind::ReversalAfterWriteOff,
        invoice_id: invoice.id,
        source_type: payment.subject_type.clone(),
        source_id: payment.subject_id,
        amount: 50_000,
        key: "payments.allocation:2".into(),
        message: "Write off more or pursue the party.".into(),
        role: ACCOUNTANT.into(),
        status: TaskStatus::Open,
        ..Default::default()
    };
    task.insert(app.db()).await.unwrap();

    // Without `receivables.view` the list and the button are closed.
    let member = person(&app, "reopen.member@test.example", MEMBER).await;
    app.acting_as(&member);
    app.get("/invoices/tasks").await.assert_forbidden();
    app.get(&format!("/invoices/tasks/{}", task.id))
        .await
        .assert_forbidden();
    app.post(&format!("/invoices/tasks/{}/done", task.id), &[])
        .await
        .assert_forbidden();

    let clerk = person(&app, "reopen.clerk@test.example", RECEIVABLES).await;
    app.acting_as(&clerk);
    app.get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see("AED 2,000.00")
        .assert_see("Removed")
        .assert_see("payment reversed")
        .assert_see("Reopened")
        .assert_see("bank return")
        .assert_see("Open tasks")
        .assert_see("Write off more or pursue the party.");
    app.get("/invoices/tasks")
        .await
        .assert_ok()
        .assert_see("Reversal tasks")
        .assert_see("Write off more or pursue the party.")
        .assert_see("INV-REOPEN-0001");
    app.get(&format!("/invoices/tasks/{}", task.id))
        .await
        .assert_redirect(&format!("/invoices/{}", invoice.id));

    // "Mark done" closes it for the user; the list and the page lose it.
    app.post(&format!("/invoices/tasks/{}/done", task.id), &[])
        .await
        .assert_ok();
    let closed = InvoicingTask::find(app.db(), task.id)
        .await
        .unwrap()
        .expect("the task");
    assert_eq!(closed.status, TaskStatus::Done);
    assert_eq!(closed.done_by, Some(clerk.id));
    assert!(closed.done_at.is_some());
    app.get("/invoices/tasks")
        .await
        .assert_ok()
        .assert_dont_see("Write off more or pursue the party.");
    app.get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_dont_see("Open tasks");
}
