//! BL-PRP-26 (#276), the deposit settings of the Property client with the
//! real modules of the host: the tenant may dispute published deductions for
//! 14 days and staff have 7 days to answer (item 17), a deduction total of
//! AED 6,000 needs a second approver (item 18) and a deposit refund recorded
//! by one user is not approved by that user (item 19).

mod support;

use host::roles::{ACCOUNTANT, FINANCE_MANAGER, PROPERTY_MANAGER};
use property::UnitUse;
use property::charge_type;
use property::deposit_forfeit::{DAMAGES, DepositSettlement};
use renox::prelude::*;
use support::*;
use wl_kernel::VatCategory;
use wl_kernel::deposits::{DisputeDeductions, deposit_deductions};
use wl_kernel::documents::today;
use wl_payments::deposit::DepositStatus;
use wl_payments::deposit_deductions::{DepositDeduction, DepositDispute};
use wl_payments::notices::DEPOSIT_DISPUTED;
use wl_payments::refund::RefundStatus;
use wl_payments::{Deposit, Refund};

/// Moves the clock `days` forward from the present.
fn travel_days(app: &renox::testing::TestApp, days: u64) {
    app.travel_back();
    app.travel(std::time::Duration::from_secs(days * 24 * 3600));
}

/// Item 17: deductions published today may be disputed by the tenant for 14
/// days; a dispute stops the refund and tells staff to answer within 7 days;
/// after the window the tenant can no longer dispute.
#[renox::test]
async fn the_tenant_may_dispute_the_deductions_for_fourteen_days() {
    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = draft_lease(&app, &unit, tenant, -300).await;
    let deposit = settling_deposit(&app, &accountant, tenant, &lease, 500_000).await;

    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/deposits/{}/deductions", deposit.id),
            &[
                ("lines", "Repainting | 1000.00 | photos-1"),
                ("forfeiture_reason", ""),
            ],
        )
        .await
        .assert_ok();
    let published = DepositDeduction::latest(app.db(), deposit.id)
        .await
        .unwrap()
        .expect("the published deductions");
    assert_eq!(published.published_on, today());
    assert_eq!(published.window_ends_on, Some(day(14)));

    // The dispute button shows: the portal reads can_dispute.
    let shown = deposit_deductions(app.state())
        .published_for(app.state(), tenant)
        .await
        .unwrap();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].can_dispute);

    // The tenant disputes the line; staff are told, with 7 days to answer.
    let request = DisputeDeductions {
        deposit: deposit.deposit_id(),
        version: 1,
        lines: vec![shown[0].lines[0].id],
        comment: "The wall was already damaged".to_owned(),
    };
    deposit_deductions(app.state())
        .dispute(app.state(), tenant, request)
        .await
        .expect("the dispute is taken");
    assert_eq!(
        DepositDispute::open_of_deposit(app.db(), deposit.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let told = messages_to(&app, DEPOSIT_DISPUTED, &accountant).await;
    assert_eq!(told.len(), 1);
    assert!(
        told[0].rendered_body.contains("7 days"),
        "{}",
        told[0].rendered_body
    );
    // The refund waits while the dispute is open.
    app.htmx()
        .post(
            &format!("/deposits/{}/refund", deposit.id),
            &[("method", "transfer")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        Deposit::find_or_404(app.db(), deposit.id)
            .await
            .unwrap()
            .status,
        DepositStatus::SettlementPending
    );

    // Fifteen days later the window is over: no dispute button.
    travel_days(&app, 15);
    let shown = app
        .at_travelled_time(deposit_deductions(app.state()).published_for(app.state(), tenant))
        .await
        .unwrap();
    assert!(!shown[0].can_dispute);
}

/// Item 18: a deduction total of AED 6,000 (above AED 5,000) applied to the
/// deposit needs a second approver, a finance manager or the owner, besides
/// the property manager; the requester approves neither.
#[renox::test]
async fn six_thousand_dirhams_of_deductions_need_a_second_approver() {
    let app = booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = draft_lease(&app, &unit, tenant, -300).await;
    charge_type::configure(app.db(), DAMAGES, "Damages", VatCategory::Standard)
        .await
        .unwrap();
    // The deposit of AED 5,000, deductions of AED 6,000 published today.
    let deposit = settling_deposit(&app, &accountant, tenant, &lease, 500_000).await;
    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/deposits/{}/deductions", deposit.id),
            &[
                ("lines", "Repainting and floors | 6000.00 | photos-1"),
                ("forfeiture_reason", "applied_to_deductions"),
            ],
        )
        .await
        .assert_ok();

    // After the 14-day window: the property manager alone is not enough.
    travel_days(&app, 15);
    let path = format!("/property/leases/{}/apply-deposit", lease.id);
    let manager_id = manager.id.to_string();
    let finance_id = finance.id.to_string();
    app.acting_as(&accountant);
    app.htmx()
        .post(&path, &[("approver", manager_id.as_str())])
        .await
        .assert_status(422);
    assert!(
        DepositSettlement::of_root(app.db(), lease.root_id)
            .await
            .unwrap()
            .is_none()
    );
    // The requester cannot be the second approver.
    let own = accountant.id.to_string();
    app.htmx()
        .post(
            &path,
            &[
                ("approver", manager_id.as_str()),
                ("second_approver", own.as_str()),
            ],
        )
        .await
        .assert_status(422);
    // With the finance manager as the second approver it is applied.
    app.htmx()
        .post(
            &path,
            &[
                ("approver", manager_id.as_str()),
                ("second_approver", finance_id.as_str()),
            ],
        )
        .await
        .assert_ok();
    let settlement = DepositSettlement::of_root(app.db(), lease.root_id)
        .await
        .unwrap()
        .expect("the deposit is applied");
    assert_eq!(settlement.damages, 600_000);
    assert_eq!(settlement.to_damages, 500_000);
    assert_eq!(settlement.excess, 100_000);
    assert_eq!(settlement.second_approver_id, Some(finance.id));
}

/// Item 19: the deposit refund recorded by accountant A waits for an approval
/// by another user; A cannot approve it, accountant B can.
#[renox::test]
async fn a_deposit_refund_is_not_approved_by_the_user_who_recorded_it() {
    let app = booted().await;
    let a = person(&app, "Amal", ACCOUNTANT).await;
    let b = person(&app, "Bilal", ACCOUNTANT).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let lease = draft_lease(&app, &unit, tenant, -300).await;
    let deposit = settling_deposit(&app, &a, tenant, &lease, 500_000).await;
    app.acting_as(&a);
    app.htmx()
        .post(
            &format!("/deposits/{}/deductions", deposit.id),
            &[("lines", ""), ("forfeiture_reason", "")],
        )
        .await
        .assert_ok();
    travel_days(&app, 15);
    app.acting_as(&a);
    app.htmx()
        .post(
            &format!("/deposits/{}/refund", deposit.id),
            &[("method", "transfer")],
        )
        .await
        .assert_ok();
    let stored = Deposit::find_or_404(app.db(), deposit.id).await.unwrap();
    assert_eq!(stored.status, DepositStatus::Refunded);
    let refund = Refund::find_or_404(app.db(), stored.refund_id.expect("a refund"))
        .await
        .unwrap();
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.amount, 500_000);

    // A recorded it: A cannot approve it (FIN.ACL.04); B can.
    let path = format!("/refunds/{}/transitions/approve", refund.id);
    app.acting_as(&a);
    let refused = app.htmx().post(&path, &[("reason", "")]).await;
    assert!(
        refused.status.is_client_error(),
        "A's approval is refused: {}",
        refused.status
    );
    assert_eq!(
        Refund::find_or_404(app.db(), refund.id)
            .await
            .unwrap()
            .status,
        RefundStatus::Requested
    );
    app.acting_as(&b);
    app.htmx()
        .post(&path, &[("reason", "")])
        .await
        .assert_status(303);
    let approved = Refund::find_or_404(app.db(), refund.id).await.unwrap();
    assert_eq!(approved.status, RefundStatus::Approved);
    assert_eq!(approved.approved_by, Some(b.id));
}
