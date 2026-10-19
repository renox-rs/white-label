//! Acceptance checks of the held possible-duplicate supplier bill (wl-invoicing
//! §4, §7, `FIN.ACL.03`, #74 part 5). Most criteria are checked next to the
//! code they cover (`supplier_bill_submit`, `supplier_bill_events`,
//! `supplier_bill_pages`); these are the ones that need the whole module: AC3
//! with an admin creator, AC8 with the daily tasks and AC10 on the page.

mod support;

use std::time::Duration;

use renox::auth::User;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::duplicates::{BillException, exceptions_of};
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{
    BillLineDraft, SupplierBill, SupplierBillStatus, bad_debt_relief, escalation, overdue,
    reminders, vat_summary,
};
use wl_kernel::invoicing::{SupplierBillId, VatCategory};

use support::{BLUE_DUNES, Booted, boot};

const B: i64 = 8;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

async fn submitted(t: &Booted, by: &User, number: &str) -> SupplierBill {
    let draft = BillDraft::new(BLUE_DUNES, number, day(5)).line(BillLineDraft::new(
        "Pump repair",
        100_000,
        VatCategory::Standard,
    ));
    let bill = supplier_bill::create(t.db(), &draft, Some(by.id))
        .await
        .unwrap();
    submit(t.state(), By::System(Some(by.id)), bill.id)
        .await
        .unwrap()
}

async fn post(
    t: &Booted,
    by: &User,
    bill: &SupplierBill,
    action: &str,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app
        .htmx()
        .post(
            &format!("/supplier-bills/{}/transitions/{action}", bill.id),
            fields,
        )
        .await
}

async fn now(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
}

/// AC3: the creator of a held duplicate cannot approve it, also as an admin
/// holding `supplier_bills.approve`; another user approves it with a reason
/// and the approval is emitted once.
#[renox::test]
async fn an_admin_creator_cannot_approve_the_held_bill_another_user_can() {
    let t = boot().await;
    let admin = t.admin_a().await;
    let ben = t.accountant("Ben").await;
    let first = submitted(&t, &admin, "INV-77").await;
    approve(t.state(), By::System(Some(B)), first.id, "")
        .await
        .unwrap();
    let before = t.bill_approved.all().len();

    let second = submitted(&t, &admin, "inv-77 ").await;
    assert_eq!(second.status, SupplierBillStatus::Exception);
    post(&t, &admin, &second, "approve", &[("reason", "mine")])
        .await
        .assert_status(422);
    assert_eq!(now(&t, &second).await.status, SupplierBillStatus::Exception);
    assert_eq!(t.bill_approved.all().len(), before);

    post(
        &t,
        &ben,
        &second,
        "approve",
        &[("reason", "two deliveries, same number")],
    )
    .await
    .assert_status(303);
    let done = now(&t, &second).await;
    assert_eq!(done.status, SupplierBillStatus::Approved);
    assert_eq!(done.approved_by, Some(ben.id));
    assert_eq!(
        done.approval_reason.as_deref(),
        Some("two deliveries, same number")
    );
    let approved = t.bill_approved.all();
    assert_eq!(approved.len(), before + 1);
    assert_eq!(approved[before].bill, SupplierBillId(second.id));
}

/// AC8: nothing in the module's daily tasks touches a held bill, also 30
/// days later.
#[renox::test]
async fn a_held_bill_is_left_alone_by_the_daily_tasks() {
    let t = boot().await;
    let a = t.user_a().await;
    let first = submitted(&t, &a, "INV-77").await;
    approve(t.state(), By::System(Some(B)), first.id, "")
        .await
        .unwrap();
    let second = submitted(&t, &a, "INV-77").await;
    assert_eq!(second.status, SupplierBillStatus::Exception);

    t.app.travel(Duration::from_secs(30 * 24 * 3600));
    for task in [
        overdue::TASK,
        reminders::TASK,
        vat_summary::TASK,
        escalation::TASK,
        bad_debt_relief::TASK,
    ] {
        t.app
            .at_travelled_time(t.app.kernel().run_scheduled(task))
            .await
            .unwrap_or_else(|e| panic!("{task} runs: {e}"));
    }
    let held = now(&t, &second).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);
    assert_eq!(held.duplicate_of, Some(first.id));
    assert_eq!(exceptions_of(t.db(), second.id).await.unwrap().len(), 1);
}

/// AC10: a second exception row shows next to the duplicate on the bill
/// page; one approval with a reason covers both, and the creator is still
/// refused.
#[renox::test]
async fn the_page_shows_every_reason_and_one_approval_covers_them() {
    let t = boot().await;
    let anna = t.accountant("Anna").await;
    let ben = t.accountant("Ben").await;
    let first = submitted(&t, &anna, "INV-77").await;
    approve(t.state(), By::System(Some(ben.id)), first.id, "")
        .await
        .unwrap();
    let second = submitted(&t, &anna, "INV-77").await;
    BillException {
        bill_id: second.id,
        reason_code: "other_reason".to_owned(),
        related_bill_id: None,
        ..Default::default()
    }
    .insert(t.db())
    .await
    .unwrap();

    t.app.acting_as(&ben);
    t.app
        .get(&format!("/supplier-bills/{}", second.id))
        .await
        .assert_ok()
        .assert_see("Possible duplicate")
        .assert_see("other_reason");

    post(&t, &anna, &second, "approve", &[("reason", "fine")])
        .await
        .assert_status(422);
    post(&t, &ben, &second, "approve", &[("reason", "checked")])
        .await
        .assert_status(303);
    let done = now(&t, &second).await;
    assert_eq!(done.status, SupplierBillStatus::Approved);
    assert_eq!(done.approved_by, Some(ben.id));
    assert_eq!(done.approval_reason.as_deref(), Some("checked"));
}
