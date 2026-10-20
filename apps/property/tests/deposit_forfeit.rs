//! BL-PRP-17 (#267): the deposit is applied in full when the deductions exceed
//! it. `forfeit` with the reason *applied to deductions* applies it to the
//! unpaid rent invoices, oldest first, then to the damage lines; the damages it
//! does not cover are invoiced to the tenant. A property manager approves, and
//! above AED 5,000 of total deductions finance or the owner approves too
//! (`PRP.ACL.05`).

mod support;

use property::deposit_forfeit::{DAMAGES, DepositSettlement};
use property::{ChargeStatus, Lease, LeaseCharge, arrears, charge_type};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::VatCategory;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, invoicer};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, TENANT, aed, boot, day};

struct Scene {
    t: Booted,
    lease: Lease,
    source: Subject,
    accountant: User,
    manager: User,
    finance: User,
    agent: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let accountant = t.user_in("Amal", ACCOUNTANT, building.id).await;
    let finance = t.user_with("Fiona", "finance_manager").await;
    let lease = t.active_lease(&unit, &agent, &manager).await;
    let source = lease.agreement();
    Scene {
        t,
        lease,
        source,
        accountant,
        manager,
        finance,
        agent,
    }
}

impl Scene {
    /// An issued rent invoice of `amount` fils due `due` days from today, as
    /// the charge of the lease it was made for.
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
        self.t.invoices.issue(id);
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

    /// The damages charge type is configured with standard VAT.
    async fn configure(&self) {
        charge_type::configure(self.t.db(), DAMAGES, "Damages", VatCategory::Standard)
            .await
            .unwrap();
    }

    /// The accountant forfeits the deposit, approved by the manager and, when
    /// given, by `second`.
    async fn forfeit(&self, second: Option<&User>) -> renox::testing::TestResponse {
        let approver = self.manager.id.to_string();
        let second = second.map(|u| u.id.to_string()).unwrap_or_default();
        self.t
            .post(
                &self.accountant,
                &format!("/property/leases/{}/apply-deposit", self.lease.id),
                &[("approver", &approver), ("second_approver", &second)],
            )
            .await
    }

    async fn settlement(&self) -> Option<DepositSettlement> {
        DepositSettlement::of_root(self.t.db(), self.lease.root_id)
            .await
            .unwrap()
    }
}

/// AC1, AC2, AC5: a deposit of AED 5,000.00, an unpaid rent invoice of AED
/// 2,000.00 and damage deductions of AED 4,500.00 (window passed, no dispute):
/// AED 2,000.00 is allocated to the rent invoice, AED 3,000.00 covers damages,
/// and a damages invoice of AED 1,500.00 is issued to the tenant. The total
/// deductions are AED 6,500.00, so a second approver is needed.
#[renox::test]
async fn bl_prp_17_the_deposit_covers_the_rent_then_the_damages_and_the_rest_is_invoiced() {
    let s = scene().await;
    s.configure().await;
    let rent = s.rent_invoice(aed(2_000), -40, 10).await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(3_000)), ("Chiller coil", aed(1_500))],
    );

    // Above AED 5,000 the manager's approval is not enough.
    s.forfeit(None).await.assert_status(422);
    // Nor is the approval of someone who is not finance or the owner.
    s.forfeit(Some(&s.agent)).await.assert_status(422);
    assert!(s.t.deposits.applied().is_empty());
    assert!(s.t.deposits.is_settling(&s.source));
    assert!(s.settlement().await.is_none());

    s.forfeit(Some(&s.finance)).await.assert_ok();

    let applied = s.t.deposits.applied();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].0.approver, s.manager.id);
    assert_eq!(applied[0].1, s.accountant.id);
    assert!(!s.t.deposits.is_settling(&s.source));

    let paid = s.t.invoices.get(rent).unwrap();
    assert_eq!(paid.summary.balance, 0);
    assert_eq!(paid.summary.status, InvoiceStatus::Paid);
    assert_eq!(
        paid.allocations
            .iter()
            .map(|(_, amount)| amount)
            .sum::<i64>(),
        aed(2_000)
    );

    let settlement = s.settlement().await.expect("a settlement statement");
    assert_eq!(settlement.held, aed(5_000));
    assert_eq!(settlement.rent_due, aed(2_000));
    assert_eq!(settlement.damages, aed(4_500));
    assert_eq!(settlement.total_deductions(), aed(6_500));
    assert_eq!(settlement.to_rent, aed(2_000));
    assert_eq!(settlement.to_damages, aed(3_000));
    assert_eq!(settlement.excess, aed(1_500));
    assert_eq!(settlement.approver_id, s.manager.id);
    assert_eq!(settlement.second_approver_id, Some(s.finance.id));

    // One damages invoice with one line for what the deposit did not cover.
    let invoice =
        s.t.invoices
            .get(settlement.excess_invoice_id.expect("a damages invoice"))
            .unwrap();
    assert_eq!(invoice.draft.party, TENANT);
    assert_eq!(invoice.draft.source, s.lease.subject());
    assert_eq!(invoice.draft.lines.len(), 1);
    assert_eq!(invoice.draft.lines[0].description, "Chiller coil");
    assert_eq!(invoice.draft.lines[0].unit_price, aed(1_500));
    assert_eq!(
        invoice.draft.lines[0].charge_type.as_deref(),
        Some("damages")
    );
    assert_eq!(invoice.draft.lines[0].vat_category, VatCategory::Standard);
    // Due after the client's standard terms: the invoicer fills the date in.
    assert!(invoice.draft.due_date.is_none());

    // It is applied once.
    s.forfeit(Some(&s.finance)).await.assert_status(422);

    // The statement is on the lease page.
    s.t.app.acting_as(&s.accountant);
    s.t.app
        .get(&format!("/property/leases/{}", s.lease.id))
        .await
        .assert_ok()
        .assert_see("Deposit settlement")
        .assert_see("AED 6,500.00")
        .assert_see("AED 1,500.00");
}

/// Only the accountant who may refund deposits forfeits one.
#[renox::test]
async fn bl_prp_17_only_a_holder_of_the_refund_permission_forfeits() {
    let s = scene().await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(5_000))],
    );
    let approver = s.manager.id.to_string();
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/apply-deposit", s.lease.id),
        &[("approver", &approver)],
    )
    .await
    .assert_forbidden();
    assert!(s.t.deposits.applied().is_empty());
}

/// AC3: with a line disputed, `forfeit` is refused; before the 14-day window
/// after publication has passed, it is refused too.
#[renox::test]
async fn bl_prp_17_a_dispute_or_an_open_window_refuses_the_forfeit() {
    let s = scene().await;
    s.configure().await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(6_000))],
    );
    s.t.deposits.dispute_line(&s.source, 1);
    let refused = s.forfeit(Some(&s.finance)).await;
    refused.assert_status(422);
    assert!(refused.text().contains("disputed"), "{}", refused.text());
    assert!(s.t.deposits.applied().is_empty());

    // Published three days ago: the tenant may still dispute.
    let open = scene().await;
    open.configure().await;
    open.t.deposits.settle_with(
        &open.source,
        TENANT,
        aed(5_000),
        -3,
        &[("Repainting", aed(6_000))],
    );
    let refused = open.forfeit(Some(&open.finance)).await;
    refused.assert_status(422);
    assert!(refused.text().contains("may dispute"), "{}", refused.text());
    assert!(open.t.deposits.applied().is_empty());
}

/// AC6: a deposit of AED 5,000.00 and unpaid rent of AED 7,000.00 (two invoices
/// of AED 3,500.00) and no damages: the older invoice is paid in full and AED
/// 1,500.00 of the newer; AED 2,000.00 stays open on it and no damages invoice
/// is issued.
#[renox::test]
async fn bl_prp_17_unpaid_rent_alone_is_paid_oldest_first_and_issues_no_invoice() {
    let s = scene().await;
    let older = s.rent_invoice(aed(3_500), -60, 10).await;
    let newer = s.rent_invoice(aed(3_500), -30, 11).await;
    s.t.deposits
        .settle_with(&s.source, TENANT, aed(5_000), -20, &[]);
    let before = s.t.invoices.all().len();

    s.forfeit(Some(&s.finance)).await.assert_ok();

    let first = s.t.invoices.get(older).unwrap();
    assert_eq!(first.summary.balance, 0);
    assert_eq!(first.summary.status, InvoiceStatus::Paid);
    let second = s.t.invoices.get(newer).unwrap();
    assert_eq!(second.summary.balance, aed(2_000));
    assert_eq!(second.summary.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(
        second
            .allocations
            .iter()
            .map(|(_, amount)| amount)
            .sum::<i64>(),
        aed(1_500)
    );

    let settlement = s.settlement().await.unwrap();
    assert_eq!(settlement.to_rent, aed(5_000));
    assert_eq!(settlement.to_damages, 0);
    assert_eq!(settlement.excess, 0);
    assert!(settlement.excess_invoice_id.is_none());
    assert_eq!(s.t.invoices.all().len(), before);
}

/// AC7: a deposit of AED 5,000.00 and damages of AED 1,200.00: `forfeit` with
/// the reason *applied to deductions* is refused (the total deductions are
/// below the deposit; the settlement goes through `refund`, which pays back AED
/// 3,800.00, BL-PAY-15).
#[renox::test]
async fn bl_prp_17_deductions_below_the_deposit_are_refused() {
    let s = scene().await;
    s.configure().await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(1_200))],
    );
    s.forfeit(None).await.assert_status(422);
    assert!(s.t.deposits.applied().is_empty());
    assert!(s.t.deposits.is_settling(&s.source));
    assert!(s.settlement().await.is_none());
}

/// Deductions exactly equal to the deposit: it is applied, no excess invoice,
/// and the manager's approval is enough (not above AED 5,000).
#[renox::test]
async fn bl_prp_17_deductions_equal_to_the_deposit_leave_no_excess() {
    let s = scene().await;
    s.configure().await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(5_000))],
    );
    s.forfeit(None).await.assert_ok();
    let settlement = s.settlement().await.unwrap();
    assert_eq!(settlement.excess, 0);
    assert!(settlement.excess_invoice_id.is_none());
    assert_eq!(settlement.to_damages, aed(5_000));
    assert!(settlement.second_approver_id.is_none());
}

/// Damage lines need the damages charge type; there is no platform default
/// while the VAT treatment is open (A-06).
#[renox::test]
async fn bl_prp_17_the_damages_charge_type_must_be_configured() {
    let s = scene().await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(5_000))],
    );
    let refused = s.forfeit(None).await;
    refused.assert_status(422);
    assert!(refused.text().contains("charge type"), "{}", refused.text());
    assert!(s.t.deposits.applied().is_empty());
}

/// AC4: the damages invoice becomes overdue and enters `PRP.RUL.10` like any
/// rent invoice.
#[renox::test]
async fn bl_prp_17_the_damages_invoice_follows_the_arrears_flow() {
    let s = scene().await;
    s.configure().await;
    s.t.deposits.settle_with(
        &s.source,
        TENANT,
        aed(5_000),
        -20,
        &[("Repainting", aed(6_500))],
    );
    s.forfeit(Some(&s.finance)).await.assert_ok();
    let settlement = s.settlement().await.unwrap();
    assert_eq!(settlement.excess, aed(1_500));
    let invoice = settlement.excess_invoice_id.unwrap();
    s.t.invoices.issue(invoice);

    let early = arrears::run(s.t.state(), day(29)).await.unwrap();
    assert_eq!(early.demands, 0);
    let late = arrears::run(s.t.state(), day(31)).await.unwrap();
    assert_eq!(late.demands, 1);
    let demands = arrears::ArrearsDemand::of_root(s.t.db(), s.lease.root_id)
        .await
        .unwrap();
    assert_eq!(demands.len(), 1);
    let listed = demands[0].invoices(s.t.db()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].invoice_id, invoice);
}
