//! BL-CON-02 (#281): settling the final account of a terminated project, and
//! cancelling only before work. A terminated project closes when its final
//! account is certified (or decided by a court or arbitration) and the final
//! account IPA is invoiced or its balance written off; `cancel` is possible
//! only from `setup`, before any attendance, goods receipt or IPA.

mod support;

use construction::final_account::closing_warnings;
use construction::{
    Guarantee, GuaranteeKind, GuaranteeStatus, Ipa, IpaKind, IpaStatus, Project, ProjectStatus,
    Retention, RetentionPart,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_attendance::service::{self, NewRecord};
use wl_inventory::goods_receipt::{DraftReceipt, create_draft};
use wl_inventory::{DraftLine, Item, PurchaseOrderLine, stock_count};
use wl_kernel::invoicing::InvoiceStatus;

use support::{Booted, boot, date};

/// A terminated project with the final account IPA in `status`, naming
/// `invoice` when it has one.
async fn terminated(
    t: &Booted,
    manager: &User,
    status: Option<IpaStatus>,
    invoice: Option<i64>,
) -> Project {
    let project = t.project(ProjectStatus::Terminated, Some(manager)).await;
    if let Some(status) = status {
        let mut account = Ipa {
            kind: IpaKind::FinalAccount,
            status,
            valuation_on: Some(date(2027, 5, 31)),
            claimed_amount: 9_000_000,
            invoice_id: invoice,
            ..Ipa::new(project.id)
        };
        if status == IpaStatus::Certified || status == IpaStatus::Invoiced {
            account.certified_amount = Some(8_500_000);
        }
        let mut tx = t.db().begin().await.unwrap();
        account
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    project
}

fn settle_path(project: &Project) -> String {
    format!("/construction/projects/{}/settle", project.id)
}

fn cancel_path(project: &Project) -> String {
    format!("/construction/projects/{}/cancel", project.id)
}

/// Done when 1: a terminated project whose final account IPA is `invoiced`
/// closes with `settle_final_account`.
#[renox::test]
async fn an_invoiced_final_account_closes_the_project() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let invoice = t.invoices.put(InvoiceStatus::Issued);
    let project = terminated(&t, &a, Some(IpaStatus::Invoiced), Some(invoice)).await;

    t.post(&a, &settle_path(&project), &[]).await.assert_ok();

    let closed = t.reload(&project).await;
    assert_eq!(closed.status, ProjectStatus::Closed);
    assert!(closed.closed_at.is_some());
}

/// Done when 2: with the final account IPA only `certified`, it is refused.
#[renox::test]
async fn a_certified_but_not_invoiced_final_account_is_refused() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let project = terminated(&t, &a, Some(IpaStatus::Certified), None).await;

    t.post(&a, &settle_path(&project), &[])
        .await
        .assert_status(422);

    assert_eq!(t.reload(&project).await.status, ProjectStatus::Terminated);
    assert!(t.reload(&project).await.closed_at.is_none());
}

/// A final account that is not certified yet, or missing, is refused too; so
/// is a project that is not terminated, and a user who does not manage it.
#[renox::test]
async fn the_final_account_needs_its_certificate_and_the_project_its_state() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    for status in [IpaStatus::Draft, IpaStatus::Submitted] {
        let project = terminated(&t, &a, Some(status), None).await;
        let refused = t.post(&a, &settle_path(&project), &[]).await;
        refused.assert_status(422);
        assert!(
            refused.text().contains("not certified"),
            "{}",
            refused.text()
        );
        assert_eq!(t.reload(&project).await.status, ProjectStatus::Terminated);
    }
    let bare = terminated(&t, &a, None, None).await;
    let refused = t.post(&a, &settle_path(&bare), &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("no final account"),
        "{}",
        refused.text()
    );

    let running = t.project(ProjectStatus::InProgress, Some(&a)).await;
    t.post(&a, &settle_path(&running), &[])
        .await
        .assert_status(422);
    let invoice = t.invoices.put(InvoiceStatus::Issued);
    let done = terminated(&t, &a, Some(IpaStatus::Invoiced), Some(invoice)).await;
    let md = t.md("Bader").await;
    t.post(&md, &settle_path(&done), &[])
        .await
        .assert_status(403);
    assert_eq!(t.reload(&done).await.status, ProjectStatus::Terminated);
}

/// Done when 3: with the final account invoice `written_off` after the MD's
/// approval, it is allowed; so is a final account certified by a recorded
/// court decision whose invoice was written off.
#[renox::test]
async fn a_written_off_final_account_invoice_lets_the_project_close() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let invoice = t.invoices.put(InvoiceStatus::WrittenOff);
    let project = terminated(&t, &a, Some(IpaStatus::Invoiced), Some(invoice)).await;
    t.post(&a, &settle_path(&project), &[]).await.assert_ok();
    assert_eq!(t.reload(&project).await.status, ProjectStatus::Closed);

    // A court decision is the certificate; the invoice is then written off
    // although the IPA still reads `certified`.
    let decided = t.invoices.put(InvoiceStatus::WrittenOff);
    let other = terminated(&t, &a, Some(IpaStatus::Certified), Some(decided)).await;
    Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(other.id))
        .update(
            t.db(),
            &[
                (
                    "decision_reference",
                    &Some("Court judgment 2027/311".to_owned()),
                ),
                ("decided_amount", &Some(7_000_000_i64)),
            ],
        )
        .await
        .unwrap();
    t.post(&a, &settle_path(&other), &[]).await.assert_ok();
    assert_eq!(t.reload(&other).await.status, ProjectStatus::Closed);

    // The same account with an invoice still open is not enough.
    let open = t.invoices.put(InvoiceStatus::Overdue);
    let unsettled = terminated(&t, &a, Some(IpaStatus::Certified), Some(open)).await;
    t.post(&a, &settle_path(&unsettled), &[])
        .await
        .assert_status(422);
}

/// Done when 6: `settle_final_account` with the site store count still open
/// succeeds after a warning; so do open retention parts and guarantees.
#[renox::test]
async fn open_counts_retention_and_guarantees_only_warn() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let invoice = t.invoices.put(InvoiceStatus::Issued);
    let project = terminated(&t, &a, Some(IpaStatus::Invoiced), Some(invoice)).await;
    let store = t.store(&project, "SITE-STORE").await;
    stock_count::open_for(t.db(), store.id, "project terminated", &project.subject())
        .await
        .unwrap();
    Retention::new(project.id, RetentionPart::Final, 5_000_000)
        .save(t.db())
        .await
        .unwrap();
    let mut guarantee = Guarantee::new(project.id, GuaranteeKind::Performance, 2_000_000);
    guarantee.status = GuaranteeStatus::Active;
    let mut tx = t.db().begin().await.unwrap();
    guarantee
        .insert_in_state(&mut tx, GuaranteeStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // The closing screen warns.
    let screen = t
        .get(
            &a,
            &format!("/construction/projects/{}/closing", project.id),
        )
        .await;
    screen.assert_ok();
    let body: renox::serde_json::Value = renox::serde_json::from_str(&screen.text()).unwrap();
    assert_eq!(body["warnings"]["stock_counts"], 1);
    assert_eq!(body["warnings"]["retention_parts"], 1);
    assert_eq!(body["warnings"]["guarantees"], 1);
    assert_eq!(body["warning_texts"].as_array().map(Vec::len), Some(3));
    assert_eq!(body["final_account"]["status"], "invoiced");
    assert_eq!(body["status"], "terminated");

    // Settling goes through and reports what was still open.
    t.post(&a, &settle_path(&project), &[]).await.assert_ok();
    assert_eq!(t.reload(&project).await.status, ProjectStatus::Closed);
    let after = closing_warnings(t.state(), project.id).await.unwrap();
    assert_eq!(after.stock_counts, 1);
    assert_eq!(after.retention_parts, 1);
    assert_eq!(after.guarantees, 1);
}

/// Done when 6 (second half): closing a completed project whose site store
/// has an open count only warns; the project closes and the answer carries the
/// stock count warning.
#[renox::test]
async fn closing_a_completed_project_with_an_open_count_only_warns() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let project = t
        .project(ProjectStatus::PracticalCompletion, Some(&a))
        .await;
    let ends = Some(date(2028, 6, 15));
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("defects_liability_ends_on", &ends)])
        .await
        .unwrap();
    let store = t.store(&project, "SITE-STORE").await;
    stock_count::open_for(t.db(), store.id, "project closing", &project.subject())
        .await
        .unwrap();
    assert_eq!(
        closing_warnings(t.state(), project.id)
            .await
            .unwrap()
            .stock_counts,
        1
    );

    let path = format!("/construction/projects/{}/close", project.id);
    let answer = t
        .on(date(2028, 6, 16), async { t.post(&a, &path, &[]).await })
        .await;
    answer.assert_ok();
    let shown = t
        .app
        .session_get::<renox::serde_json::Value>("_toasts")
        .map(|toasts| toasts.to_string())
        .unwrap_or_default();
    assert!(
        shown.contains("stock count"),
        "the close warns about the open count: {shown}"
    );
    assert_eq!(t.reload(&project).await.status, ProjectStatus::Closed);
    // The count is still open: it only warned.
    assert_eq!(
        closing_warnings(t.state(), project.id)
            .await
            .unwrap()
            .stock_counts,
        1
    );
}

/// A goods receipt at the store of `project`.
async fn receipt_at(t: &Booted, project: &Project) {
    let store = t.store(project, "SITE-STORE").await;
    let order = t.order(&store, "PO-2027-0100").await;
    let mut item = Item::new("CEMENT", "Cement", "bag");
    item.save(t.db()).await.unwrap();
    let mut line = PurchaseOrderLine::new(order.id, 1, item.id, 10_000, 1_800);
    line.save(t.db()).await.unwrap();
    create_draft(
        t.db(),
        None,
        DraftReceipt {
            purchase_order_id: order.id,
            location_id: store.id,
            delivery_note: Some("DN-1".into()),
            lines: vec![DraftLine::on(line.id, 10_000, 10_000)],
        },
    )
    .await
    .unwrap();
}

/// Done when 4: a `setup` project with one goods receipt cannot be cancelled;
/// without any, the MD cancels it with a reason and its 2 POs are flagged for
/// cancellation.
#[renox::test]
async fn the_md_cancels_a_project_before_work_and_its_orders_are_flagged() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;

    let worked = t.project(ProjectStatus::Setup, Some(&a)).await;
    receipt_at(&t, &worked).await;
    let refused = t
        .post(&md, &cancel_path(&worked), &[("reason", "client changed")])
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("goods receipt"),
        "{}",
        refused.text()
    );
    assert_eq!(t.reload(&worked).await.status, ProjectStatus::Setup);

    let mut fresh = Project::new("VIL-02", "Palm villa fit-out", support::CLIENT_PARTY)
        .with_budget(10_000_000)
        .managed_by(a.id);
    let mut tx = t.db().begin().await.unwrap();
    fresh
        .insert_in_state(&mut tx, ProjectStatus::Setup, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let store = t.store(&fresh, "VILLA-STORE").await;
    t.order(&store, "PO-2027-0201").await;
    t.order(&store, "PO-2027-0202").await;

    // Only the MD cancels, and with a reason.
    t.post(&a, &cancel_path(&fresh), &[("reason", "client changed")])
        .await
        .assert_status(403);
    t.post(&md, &cancel_path(&fresh), &[("reason", "  ")])
        .await
        .assert_status(422);
    assert_eq!(t.reload(&fresh).await.status, ProjectStatus::Setup);

    t.post(
        &md,
        &cancel_path(&fresh),
        &[("reason", "client withdrew the award")],
    )
    .await
    .assert_ok();

    let cancelled = t.reload(&fresh).await;
    assert_eq!(cancelled.status, ProjectStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("client withdrew the award")
    );
    let flagged = t.tasks.with_key("construction.cancel_orders:");
    assert_eq!(flagged.len(), 1);
    assert!(flagged[0].ticket.description.contains("PO-2027-0201"));
    assert!(flagged[0].ticket.description.contains("PO-2027-0202"));
}

/// Done when 5: `cancel` from `in_progress` is not offered, and once a
/// project is terminated or closed it is not either; attendance or an IPA
/// refuses it as well.
#[renox::test]
async fn cancel_is_only_for_a_project_that_has_not_started() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let md = t.md("Bader").await;
    let boss = t.user_with("Bashir", "md").await;

    let running = t.project(ProjectStatus::InProgress, Some(&a)).await;
    t.post(
        &md,
        &cancel_path(&running),
        &[("reason", "no longer needed")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload(&running).await.status, ProjectStatus::InProgress);
    let machine = Project::machine();
    let offered = machine.find(construction::project::CANCEL).unwrap();
    assert_eq!(offered.from_states(), ["setup"]);

    // An IPA already exists for a project still in setup.
    let with_ipa = t.project(ProjectStatus::Setup, Some(&a)).await;
    Ipa::new(with_ipa.id).save(t.db()).await.unwrap();
    let refused = t
        .post(
            &md,
            &cancel_path(&with_ipa),
            &[("reason", "no longer needed")],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("IPA"), "{}", refused.text());

    // So does attendance on a site of the project.
    let attended = t.project(ProjectStatus::Setup, Some(&a)).await;
    let site = t.work_location(&attended).await;
    let worker = t.staff.employ(1);
    service::record(
        t.state(),
        NewRecord::supervisor(worker, site.id, renox::db::now(), None, boss.id),
    )
    .await
    .unwrap();
    let refused = t
        .post(
            &md,
            &cancel_path(&attended),
            &[("reason", "no longer needed")],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("Attendance"), "{}", refused.text());
    assert_eq!(t.reload(&attended).await.status, ProjectStatus::Setup);
}
