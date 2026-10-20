//! BL-PRP-08 (#258) with the real wl-payments for the deposit (no deposit book
//! or refund fake): a changed deposit runs `adjust` on the real deposit, which
//! stays `held`; an increase raises a top-up charge and the held amount rises
//! when it is paid, a decrease asks a part refund and the held amount falls
//! when it is paid; `DepositAdjusted` is emitted. Also the Ejari task of the
//! amendment and an amendment of an `expired` lease.

mod support;

use property::amend::DEPOSIT_TOP_UP;
use property::leases::NewVersion;
use property::renewal;
use property::{Lease, LeaseCharge, LeaseStatus, NewLease};
use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::deposits::deposits;
use wl_kernel::payments::{
    AllocationTarget, PaymentMethod, RecordPayment, RefundSource, payment_recorder,
};
use wl_payments::deposit::DepositStatus;
use wl_payments::deposit_adjust::{AdjustmentStatus, DepositAdjustment};
use wl_payments::refund::RefundStatus;
use wl_payments::{Allocation, Deposit, Refund};

use support::{AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with, d};

struct Scene {
    t: Booted,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    scene_with(Real::PAYMENTS).await
}

async fn scene_with(real: Real) -> Scene {
    let t = boot_with(real).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        unit,
        agent,
        manager,
    }
}

fn ymd(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

impl Scene {
    /// An `active` lease 2027-01-01 to 2027-12-31 at AED 120,000 a year in 12
    /// monthly charges; its deposit of AED 5,000 is held by wl-payments.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(120_000), 12)
            .deposit(aed(5_000));
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.hold_real_deposit(&lease, aed(5_000)).await;
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.reload(&lease).await
    }

    /// A submitted amendment of `v1` from `effective` to 2028-04-30.
    async fn amendment(&self, v1: &Lease, effective: NaiveDate, deposit: i64) -> Lease {
        let new = NewVersion::amendment(effective, ymd(2028, 4, 30), aed(120_000), deposit, 12);
        let v2 = property::leases::create_version(self.t.state(), Some(self.agent.id), v1.id, new)
            .await
            .unwrap();
        self.t.accept(&v2);
        self.t.submit(&self.agent, &v2).await.assert_ok();
        self.t.reload(&v2).await
    }

    async fn deposit(&self, lease: &Lease) -> Deposit {
        Deposit::query()
            .where_(Deposit::SOURCE_ID.eq(lease.agreement().subject_id))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the deposit")
    }

    async fn adjustments(&self, deposit: &Deposit) -> Vec<DepositAdjustment> {
        DepositAdjustment::of_deposit(self.t.db(), deposit.id)
            .await
            .unwrap()
    }

    async fn top_ups(&self, lease: &Lease) -> Vec<LeaseCharge> {
        lease
            .charges(self.t.db())
            .await
            .unwrap()
            .into_iter()
            .filter(|c| c.charge_type == DEPOSIT_TOP_UP)
            .collect()
    }

    /// A user holding exactly the refund permission `grant`.
    async fn clerk(&self, name: &str, grant: &str) -> User {
        self.t.user_with_grants(name, &[grant]).await
    }
}

/// AC4, raised: the deposit goes from AED 5,000 to AED 6,000 with an amendment
/// effective 2027-05-01. The real deposit stays `held` with AED 5,000 and a
/// required amount of AED 6,000; a top-up charge of AED 1,000 is dated
/// 2027-05-01; `DepositAdjusted` says so; once the tenant's AED 1,000 is
/// allocated to the deposit the held amount is AED 6,000 and the adjustment is
/// settled.
#[renox::test]
async fn bl_prp_08_a_raised_deposit_stays_held_and_the_top_up_raises_the_held_amount() {
    let s = scene().await;
    let v1 = s.leased().await;
    let v2 = s.amendment(&v1, d(5, 1), aed(6_000)).await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    let deposit = s.deposit(&v1).await;
    assert_eq!(deposit.status, DepositStatus::Held);
    assert_eq!(deposit.amount, aed(6_000));
    assert_eq!(deposit.held_amount, aed(5_000));
    let top_up = s.top_ups(&v2).await;
    assert_eq!(top_up.len(), 1);
    assert_eq!(top_up[0].amount, aed(1_000));
    assert_eq!(top_up[0].due_date, d(5, 1));
    let events = s.t.deposit_adjusted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, v1.agreement());
    assert_eq!(events[0].party, TENANT);
    assert_eq!(events[0].old_amount, aed(5_000));
    assert_eq!(events[0].new_amount, aed(6_000));
    assert_eq!(events[0].source_version, 2);
    assert_eq!(events[0].effective_on, d(5, 1));
    assert_eq!(events[0].top_up_charge, Some(top_up[0].subject()));
    assert_eq!(events[0].part_refund, None);
    assert!(s.t.tasks.with_key("property.deposit_adjust").is_empty());
    assert_eq!(
        s.adjustments(&deposit).await[0].status,
        AdjustmentStatus::Open
    );

    // The tenant pays the top-up: the payment is allocated to the deposit.
    let entity = wl_kernel::LegalEntity::query()
        .where_(wl_kernel::LegalEntity::CODE.eq("PGP"))
        .first(s.t.db())
        .await
        .unwrap()
        .expect("the default entity");
    let mut request = RecordPayment::new(TENANT, entity.id, PaymentMethod::Transfer, aed(1_000));
    request.reference = Some("TRF-TOPUP".to_owned());
    let payment = payment_recorder(s.t.state())
        .record(s.t.state(), request, None)
        .await
        .expect("the payment is recorded");
    payment_recorder(s.t.state())
        .allocate(
            s.t.state(),
            payment.id,
            AllocationTarget::Deposit(deposit.deposit_id()),
            aed(1_000),
        )
        .await
        .expect("the payment is allocated to the deposit");

    let paid = s.deposit(&v1).await;
    assert_eq!(paid.held_amount, aed(6_000));
    assert_eq!(paid.status, DepositStatus::Held);
    assert_eq!(
        s.adjustments(&paid).await[0].status,
        AdjustmentStatus::Settled
    );
    let held = deposits(s.t.state())
        .held_for(s.t.db(), &v1.agreement())
        .await
        .unwrap()
        .expect("still held");
    assert_eq!(held.held, aed(6_000));
}

/// AC4, lowered: from AED 5,000 to AED 4,000 asks a part refund of AED 1,000
/// with the deposit as its source; the held amount stays AED 5,000 until it is
/// paid and is AED 4,000 after, and the deposit stays `held`.
#[renox::test]
async fn bl_prp_08_a_lowered_deposit_stays_held_and_the_paid_refund_lowers_the_held_amount() {
    let s = scene().await;
    let v1 = s.leased().await;
    let v2 = s.amendment(&v1, d(5, 1), aed(4_000)).await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    let deposit = s.deposit(&v1).await;
    assert_eq!(deposit.status, DepositStatus::Held);
    assert_eq!(deposit.amount, aed(4_000));
    assert_eq!(
        deposit.held_amount,
        aed(5_000),
        "not lowered before payment"
    );
    assert!(s.top_ups(&v2).await.is_empty());
    let refunds = Refund::query()
        .where_(Refund::PARTY_ID.eq(TENANT.0))
        .get(s.t.db())
        .await
        .unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(1_000));
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(
        refunds[0].source(),
        Some(RefundSource::Deposit(deposit.deposit_id()))
    );
    let events = s.t.deposit_adjusted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].old_amount, aed(5_000));
    assert_eq!(events[0].new_amount, aed(4_000));
    assert_eq!(events[0].part_refund, Some(refunds[0].refund_id()));
    assert_eq!(events[0].top_up_charge, None);

    // Another user approves it and a third pays it.
    let approver = s
        .clerk("Rana", wl_payments::permissions::REFUNDS_APPROVE)
        .await;
    let payer = s.clerk("Paul", wl_payments::permissions::REFUNDS_PAY).await;
    let path = format!("/test/refunds/{}", refunds[0].id);
    s.t.post(&approver, &format!("{path}/approve"), &[])
        .await
        .assert_ok();
    assert_eq!(s.deposit(&v1).await.held_amount, aed(5_000));
    s.t.post(
        &payer,
        &format!("{path}/pay"),
        &[("method", "transfer"), ("reference", "TRF-PART")],
    )
    .await
    .assert_ok();

    let paid = s.deposit(&v1).await;
    assert_eq!(paid.held_amount, aed(4_000));
    assert_eq!(paid.status, DepositStatus::Held);
    assert_eq!(
        s.adjustments(&paid).await[0].status,
        AdjustmentStatus::Settled
    );
    let held = deposits(s.t.state())
        .held_for(s.t.db(), &v1.agreement())
        .await
        .unwrap()
        .expect("still held");
    assert_eq!(held.held, aed(4_000));
}

/// PRP.RUL.02: the Ejari task of the amendment is due 14 days after its
/// activation, and recording the Ejari number closes it, so no reminder
/// follows.
#[renox::test]
async fn bl_prp_08_the_ejari_task_of_the_amendment_is_due_in_fourteen_days_and_closes_when_recorded()
 {
    let s = scene().await;
    let v1 = s.leased().await;
    let v2 = s.amendment(&v1, d(5, 1), aed(5_000)).await;

    let before = renox::db::now();
    s.t.activate(&s.manager, &v2).await.assert_ok();
    let after = renox::db::now();

    let key = format!("property.ejari:{}", v2.id);
    let tasks = s.t.tasks.with_key(&key);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open);
    let due = tasks[0].due_at.expect("a due time");
    assert!(
        due >= before + Duration::days(14) && due <= after + Duration::days(14),
        "{due} is not 14 days after the activation"
    );

    // The property manager records the number: the task closes.
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/ejari", v2.id),
        &[("number", "EJ-7001"), ("registered_on", "2027-05-03")],
    )
    .await
    .assert_ok();
    assert_eq!(
        s.t.reload(&v2).await.ejari_number.as_deref(),
        Some("EJ-7001")
    );
    let tasks = s.t.tasks.with_key(&key);
    assert_eq!(tasks.len(), 1);
    assert!(!tasks[0].open, "{:?}", tasks[0]);
}

/// The machine lets an `expired` lease be amended (`active`, `renewal_due`,
/// `expired --amend--> amended`): an amendment effective inside the term the
/// lease had is activated and v1 becomes `amended`.
#[renox::test]
async fn bl_prp_08_an_expired_lease_can_be_amended() {
    let s = scene().await;
    let v1 = s.leased().await;
    renewal::run(s.t.state(), d(9, 2)).await.unwrap();
    renewal::run(s.t.state(), ymd(2028, 1, 1)).await.unwrap();
    assert_eq!(s.t.status(&v1).await, LeaseStatus::Expired);
    let v2 = s.amendment(&v1, d(12, 1), aed(5_000)).await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    let old = s.t.reload(&v1).await;
    assert_eq!(old.status, LeaseStatus::Amended);
    assert_eq!(old.next_id, Some(v2.id));
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Active);
    assert_eq!(s.t.amended.all().len(), 1);
}

/// AC3 with the real invoicing and payments: the April invoice of AED 10,000 is
/// issued and paid; an amendment effective 2027-04-15 credits it and reissues
/// it as two lines (14 days of the old rent, 16 of the new). The tenant's
/// payment is taken off the old invoice (the invoice is `credited`; its
/// allocation is removed on both sides and the AED 10,000 is the tenant's
/// credit again). The kernel has no call that issues an invoice, so the
/// reissued one is a draft that cannot take the money yet: the accountant gets
/// a task, issues it and allocates the credit to it, and it is then open for
/// the AED 533.34 more.
#[renox::test]
async fn bl_prp_08_the_payment_moves_from_the_credited_invoice_to_the_reissued_one() {
    use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

    let s = scene_with(Real::MONEY).await;
    let v1 = s.leased().await;
    property::invoicing::run(s.t.state(), d(4, 1))
        .await
        .unwrap();
    let mut charges = v1.charges(s.t.db()).await.unwrap();
    charges.sort_by_key(|c| (c.due_date, c.id));
    let april = charges
        .iter()
        .find(|c| c.due_date == d(4, 1))
        .expect("the April charge");
    let old = april.invoice_id.expect("the April invoice");
    renox::auth::permissions::define_role(
        s.t.db(),
        "invoice_clerk",
        &[
            wl_invoicing::permissions::VIEW,
            wl_invoicing::permissions::ISSUE,
        ],
    )
    .await
    .unwrap();
    let clerk = s.t.user_with("Clara", "invoice_clerk").await;
    for charge in charges.iter().filter(|c| c.invoice_id.is_some()) {
        let id = charge.invoice_id.unwrap();
        let issued =
            s.t.post(&clerk, &format!("/invoices/{id}/issue"), &[])
                .await;
        assert!(issued.status.is_redirection(), "{}", issued.status);
    }
    let entity = wl_kernel::LegalEntity::query()
        .where_(wl_kernel::LegalEntity::CODE.eq("PGP"))
        .first(s.t.db())
        .await
        .unwrap()
        .expect("the default entity");
    let mut request = RecordPayment::new(TENANT, entity.id, PaymentMethod::Transfer, aed(10_000));
    request.reference = Some("TRF-APRIL".to_owned());
    let payment = payment_recorder(s.t.state())
        .record(s.t.state(), request, None)
        .await
        .unwrap();
    payment_recorder(s.t.state())
        .allocate(
            s.t.state(),
            payment.id,
            AllocationTarget::Invoice(InvoiceId(old)),
            aed(10_000),
        )
        .await
        .unwrap();
    let paid = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(old))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paid.status, InvoiceStatus::Paid);
    // The new plan starts with the May charge.
    let new = NewVersion {
        first_due: Some(d(5, 1)),
        ..NewVersion::amendment(d(4, 15), ymd(2028, 4, 30), aed(132_000), aed(5_000), 12)
    };
    let v2 = property::leases::create_version(s.t.state(), Some(s.agent.id), v1.id, new)
        .await
        .unwrap();
    s.t.accept(&v2);
    s.t.submit(&s.agent, &v2).await.assert_ok();

    s.t.activate(&s.manager, &v2).await.assert_ok();

    // The old invoice is credited and the payment no longer pays it, on the
    // invoice's side and on the payment's side.
    let credited = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(old))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credited.status, InvoiceStatus::Credited);
    let invoice_side = wl_invoicing::Allocation::query()
        .where_(wl_invoicing::Allocation::INVOICE_ID.eq(old))
        .get(s.t.db())
        .await
        .unwrap();
    assert!(!invoice_side.is_empty());
    assert!(
        invoice_side.iter().all(|a| !a.is_active()),
        "{invoice_side:?}"
    );
    let rows = Allocation::all_of(s.t.db(), payment.id.0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_active(), "{rows:?}");
    assert_eq!(rows[0].removed_reason.as_deref(), Some("allocation_moved"));
    let summary = payment_recorder(s.t.state())
        .find(s.t.db(), payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.unallocated, aed(10_000), "the tenant's credit");

    // The reissued invoice has the two periods; it is a draft, so the credit
    // waits and the accountant is given the step.
    let charges = v1.charges(s.t.db()).await.unwrap();
    let reissued = charges
        .iter()
        .find(|c| c.id == april.id)
        .and_then(|c| c.invoice_id)
        .expect("the charge stands on the new invoice");
    assert_ne!(reissued, old);
    let new_invoice = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(reissued))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new_invoice.status, InvoiceStatus::Draft);
    assert_eq!(new_invoice.gross_total, 1_053_334);
    let todo =
        s.t.tasks
            .with_key(&format!("property.amend_problems:{}", v2.id));
    assert_eq!(todo.len(), 1);
    assert!(todo[0].description.contains("stays as the tenant's credit"));

    // The clerk issues it and the accountant allocates the credit to it.
    let issued =
        s.t.post(&clerk, &format!("/invoices/{reissued}/issue"), &[])
            .await;
    assert!(issued.status.is_redirection(), "{}", issued.status);
    payment_recorder(s.t.state())
        .allocate(
            s.t.state(),
            payment.id,
            AllocationTarget::Invoice(InvoiceId(reissued)),
            aed(10_000),
        )
        .await
        .expect("the credit is allocated to the reissued invoice");
    let new_invoice = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(reissued))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new_invoice.balance, 53_334);
    assert_eq!(new_invoice.status, InvoiceStatus::PartiallyPaid);
}
