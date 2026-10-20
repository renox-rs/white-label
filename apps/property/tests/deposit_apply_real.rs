//! BL-PRP-17 (#267) with the real wl-payments and wl-invoicing (no deposit
//! book or invoicer fake): `forfeit` with the reason *applied to deductions*
//! keeps the deposit and the real modules allocate it to the unpaid rent
//! invoices as payments of method *deposit applied*, oldest first; the damages
//! the deposit did not cover are invoiced to the tenant.
//!
//! The damages invoice is left a draft: the kernel `Invoicer` has no call that
//! issues an invoice (wl-invoicing issues it on its own route), so the clerk
//! issues it there and the statement of the lease page then shows its number.

mod support;

use property::deposit_forfeit::{DAMAGES, DepositSettlement};
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, charge_type};
use renox::chrono::Duration as Days;
use renox::prelude::*;
use wl_kernel::VatCategory;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::deposits::{APPLIED_TO_DEDUCTIONS, deposit_deductions, deposits};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::PaymentMethod;
use wl_payments::deposit::DepositStatus;
use wl_payments::{Allocation, Deposit, Payment};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with, day};

struct Scene {
    t: Booted,
    lease: Lease,
    accountant: User,
    manager: User,
    finance: User,
    teller: User,
    clerk: User,
}

/// A lease whose deposit of AED 5,000 is held by the real wl-payments and
/// whose settlement began (the agreement was cancelled, the first version of
/// the deductions published empty).
async fn scene() -> Scene {
    let t = boot_with(Real::MONEY).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let finance = t.user_with("Fiona", "finance_manager").await;
    let lease = t.lease(&unit, 0).await;
    t.hold_real_deposit(&lease, aed(5_000)).await;
    t.submit(&agent, &lease).await.assert_ok();
    let lease = t.reload(&lease).await;
    t.post(
        &agent,
        &format!("/property/leases/{}/cancel", lease.id),
        &[("reason", "prospect backed out")],
    )
    .await
    .assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &lease.subject())
        .await
        .unwrap()
        .remove(0);
    approve(t.state(), request.id, &manager, "ok")
        .await
        .unwrap()
        .expect("the final approval");
    assert_eq!(t.status(&lease).await, LeaseStatus::Cancelled);

    renox::auth::permissions::define_role(
        t.db(),
        "deposit_publisher",
        &[wl_payments::permissions::DEPOSITS_MANAGE],
    )
    .await
    .unwrap();
    let teller = t.user_with("Tessa", "deposit_publisher").await;
    renox::auth::permissions::define_role(
        t.db(),
        "invoice_clerk",
        &[
            wl_invoicing::permissions::VIEW,
            wl_invoicing::permissions::ISSUE,
        ],
    )
    .await
    .unwrap();
    let clerk = t.user_with("Clara", "invoice_clerk").await;
    charge_type::configure(t.db(), DAMAGES, "Damages", VatCategory::Standard)
        .await
        .unwrap();
    Scene {
        t,
        lease,
        accountant,
        manager,
        finance,
        teller,
        clerk,
    }
}

impl Scene {
    /// An issued rent invoice of `amount` fils due `due` days from today, as
    /// the charge of the lease, made and issued by the real invoicing.
    async fn rent_invoice(&self, amount: i64, due: i64, position: i32) -> i64 {
        let draft = InvoiceDraft::new(TENANT, self.lease.subject())
            .due_date(day(due))
            .line(DraftLine::new("Rent", amount, VatCategory::Exempt));
        let id = invoicer(self.t.state())
            .draft(self.t.db(), draft)
            .await
            .unwrap()
            .expect("invoicing is on")
            .0;
        self.issue(id).await;
        LeaseCharge::create(
            self.t.db(),
            LeaseCharge {
                lease_id: self.lease.id,
                position,
                charge_type: "rent".to_owned(),
                description: format!("Rent {position}"),
                due_date: day(due),
                amount,
                vat_category: VatCategory::Exempt,
                status: ChargeStatus::Invoiced,
                invoice_id: Some(id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        id
    }

    /// The clerk issues the invoice `id` on its route.
    async fn issue(&self, id: i64) {
        let issued = self
            .t
            .post(&self.clerk, &format!("/invoices/{id}/issue"), &[])
            .await;
        assert!(
            issued.status.is_success() || issued.status.is_redirection(),
            "{}: {}",
            issued.status,
            issued.text()
        );
    }

    /// The teller publishes the damage `lines` (`description | amount |
    /// evidence` per row) as the next version of the deductions.
    async fn publish(&self, lines: &str) {
        let deposit = self.deposit().await;
        self.t
            .post(
                &self.teller,
                &format!("/deposits/{}/deductions", deposit.id),
                &[
                    ("lines", lines),
                    ("forfeiture_reason", APPLIED_TO_DEDUCTIONS),
                ],
            )
            .await
            .assert_ok();
    }

    async fn deposit(&self) -> Deposit {
        Deposit::query()
            .where_(Deposit::SOURCE_TYPE.eq(self.lease.agreement().subject_type.as_str()))
            .where_(Deposit::SOURCE_ID.eq(self.lease.agreement().subject_id))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the deposit")
    }

    /// The accountant forfeits the deposit once the window of the published
    /// version has passed, approved by the manager and by finance.
    async fn forfeit(&self) -> renox::testing::TestResponse {
        let approver = self.manager.id.to_string();
        let second = self.finance.id.to_string();
        let path = format!("/property/leases/{}/apply-deposit", self.lease.id);
        self.t
            .at(
                today() + Days::days(15),
                self.t.post(
                    &self.accountant,
                    &path,
                    &[("approver", &approver), ("second_approver", &second)],
                ),
            )
            .await
    }

    /// The payments the deposit made: `(reference, amount)` of the payments of
    /// method *deposit applied*, with the invoice each one is allocated to.
    async fn applied_payments(&self) -> Vec<(i64, i64)> {
        let payments = Payment::query()
            .where_(Payment::METHOD.eq(PaymentMethod::SetOff))
            .order_by(Payment::ID)
            .get(self.t.db())
            .await
            .unwrap();
        let mut found = Vec::new();
        for payment in payments {
            assert_eq!(payment.reference.as_deref(), Some("Deposit applied"));
            let allocations = Allocation::query()
                .where_(Allocation::PAYMENT_ID.eq(payment.id))
                .get(self.t.db())
                .await
                .unwrap();
            assert_eq!(allocations.len(), 1, "one invoice per payment");
            assert_eq!(allocations[0].target_type, "invoicing.invoice");
            assert_eq!(allocations[0].amount, payment.amount);
            found.push((allocations[0].target_id, payment.amount));
        }
        found
    }

    async fn summary(&self, invoice: i64) -> wl_kernel::invoicing::InvoiceSummary {
        invoicer(self.t.state())
            .find(self.t.db(), InvoiceId(invoice))
            .await
            .unwrap()
            .expect("the invoice")
    }
}

/// AC1, AC2: a deposit of AED 5,000.00, an unpaid rent invoice of AED 2,000.00
/// and damage deductions of AED 4,500.00: the real payments allocate AED
/// 2,000.00 to the rent invoice as a payment of method *deposit applied*, AED
/// 3,000.00 cover damages, the deposit is `forfeited`, and a damages invoice of
/// AED 1,500.00 is made (a draft) for the tenant. Once the clerk issues it the
/// statement on the lease page shows its number.
#[renox::test]
async fn bl_prp_17_the_real_modules_apply_the_deposit_to_the_rent_and_the_damages() {
    let s = scene().await;
    let rent = s.rent_invoice(aed(2_000), -40, 10).await;
    s.publish("Repainting | 3000.00 | photos-1\nChiller coil | 1500.00 | photos-2")
        .await;
    let published = deposit_deductions(s.t.state())
        .published_for(s.t.state(), TENANT)
        .await
        .unwrap();
    assert_eq!(published[0].version, 2);
    assert_eq!(published[0].lines.len(), 2);

    // The window of the second version is still open on the day it is published.
    s.t.post(
        &s.accountant,
        &format!("/property/leases/{}/apply-deposit", s.lease.id),
        &[
            ("approver", &s.manager.id.to_string()),
            ("second_approver", &s.finance.id.to_string()),
        ],
    )
    .await
    .assert_status(422);
    assert_eq!(s.deposit().await.status, DepositStatus::SettlementPending);
    assert!(s.applied_payments().await.is_empty());

    s.forfeit().await.assert_ok();

    // The real wl-payments forfeited the deposit and allocated it to the rent.
    let deposit = s.deposit().await;
    assert_eq!(deposit.status, DepositStatus::Forfeited);
    assert_eq!(s.applied_payments().await, vec![(rent, aed(2_000))]);
    let paid = s.summary(rent).await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(paid.balance, 0);
    assert!(
        deposits(s.t.state())
            .held_for(s.t.db(), &s.lease.agreement())
            .await
            .unwrap()
            .is_none()
    );

    let settlement = DepositSettlement::of_root(s.t.db(), s.lease.root_id)
        .await
        .unwrap()
        .expect("a settlement statement");
    assert_eq!(settlement.held, aed(5_000));
    assert_eq!(settlement.to_rent, aed(2_000));
    assert_eq!(settlement.to_damages, aed(3_000));
    assert_eq!(settlement.excess, aed(1_500));

    // The damages invoice is a draft of AED 1,500.00 to the tenant.
    let invoice = settlement.excess_invoice_id.expect("a damages invoice");
    let drafted = s.summary(invoice).await;
    assert_eq!(drafted.status, InvoiceStatus::Draft);
    assert_eq!(drafted.party, TENANT);
    assert!(drafted.number.is_none());
    let lease_page = format!("/property/leases/{}", s.lease.id);
    s.t.app.acting_as(&s.accountant);
    s.t.app
        .get(&lease_page)
        .await
        .assert_ok()
        .assert_see("Deposit settlement")
        .assert_see("AED 6,500.00")
        .assert_see("AED 1,500.00")
        .assert_see(&format!("#{invoice}"));

    // The clerk issues it; the statement then shows the invoice number.
    s.issue(invoice).await;
    let issued = s.summary(invoice).await;
    assert_eq!(issued.status, InvoiceStatus::Issued);
    assert_eq!(issued.balance, aed(1_500) + aed(1_500) * 5 / 100);
    let number = issued.number.expect("an invoice number");
    s.t.app.acting_as(&s.accountant);
    let response = s.t.app.get(&lease_page).await;
    response.assert_ok().assert_see(&number);
    assert!(!response.text().contains(&format!("#{invoice}<")));

    // It is applied once.
    s.forfeit().await.assert_status(422);
    assert_eq!(s.applied_payments().await.len(), 1);
}

/// AC6: a deposit of AED 5,000.00 and unpaid rent of AED 7,000.00 (two invoices
/// of AED 3,500.00), no damages: the real payments pay the older invoice in
/// full and AED 1,500.00 of the newer, AED 2,000.00 stays open on it and no
/// invoice is made.
#[renox::test]
async fn bl_prp_17_the_real_payments_pay_the_oldest_rent_first() {
    let s = scene().await;
    let older = s.rent_invoice(aed(3_500), -60, 10).await;
    let newer = s.rent_invoice(aed(3_500), -30, 11).await;
    s.publish("").await;
    let invoices_before = wl_invoicing::Invoice::query()
        .count(s.t.db())
        .await
        .unwrap();

    s.forfeit().await.assert_ok();

    assert_eq!(s.deposit().await.status, DepositStatus::Forfeited);
    assert_eq!(
        s.applied_payments().await,
        vec![(older, aed(3_500)), (newer, aed(1_500))]
    );
    let first = s.summary(older).await;
    assert_eq!(first.status, InvoiceStatus::Paid);
    assert_eq!(first.balance, 0);
    let second = s.summary(newer).await;
    assert_eq!(second.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(second.balance, aed(2_000));

    let settlement = DepositSettlement::of_root(s.t.db(), s.lease.root_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settlement.to_rent, aed(5_000));
    assert_eq!(settlement.to_damages, 0);
    assert_eq!(settlement.excess, 0);
    assert!(settlement.excess_invoice_id.is_none());
    assert_eq!(
        wl_invoicing::Invoice::query()
            .count(s.t.db())
            .await
            .unwrap(),
        invoices_before,
        "no invoice is made"
    );
}
