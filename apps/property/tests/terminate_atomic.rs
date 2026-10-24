//! BL-PRP-33 (#478) with the real wl-invoicing (the cheques and the deposit are
//! the fakes): `terminate` and its money effects succeed together, or a step
//! that would fail refuses `terminate` with its reason and nothing changes.
//!
//! `terminate` runs the money effects in its own transaction through the
//! contracts' transaction-aware variants (BL-KRN-11), and checks first the
//! steps that can be known to fail, so a refused `terminate` leaves the lease,
//! the unit, the invoices, the cheques and the deposit as they were.
//!
//! The lease runs 2027-01-01 to 2027-12-31 at AED 9,000 a month with 12
//! monthly cheques; the tenant leaves on 2027-06-20, so June (10 of 30 days)
//! and July (in full) are credited and the 5 cheques of August to December go
//! back.

mod support;

use property::{ChargeStatus, Lease, LeaseStatus, NewLease, UnitStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::cheques::ChequeStatus;
use wl_kernel::invoicing::{CreditLine, InvoiceId, invoicer};

use support::{AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with, d};

struct Scene {
    t: Booted,
    unit: property::Unit,
    manager: User,
    clerk: User,
    lease: Lease,
}

async fn scene() -> Scene {
    let t = boot_with(Real::INVOICING).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
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
    let new = NewLease::new(unit.id, TENANT, d(1, 1), d(12, 31), aed(108_000), 12);
    let lease = property::leases::create_draft(t.state(), None, new)
        .await
        .unwrap();
    t.submit(&agent, &lease).await.assert_ok();
    t.activate(&manager, &lease).await.assert_ok();
    t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
    for month in 1..=12 {
        t.cheques.receive(
            TENANT,
            &lease.agreement(),
            &format!("{:06}", 100 + month),
            d(month, 1),
            aed(9_000),
            &[],
        );
    }
    let lease = t.reload(&lease).await;
    Scene {
        t,
        unit,
        manager,
        clerk,
        lease,
    }
}

impl Scene {
    /// The invoices due by July are drafted; all are issued and paid except
    /// those due on `left_draft`, which stay drafts.
    async fn invoices(&self, left_draft: Option<NaiveDate>) {
        property::invoicing::run(self.t.state(), d(7, 1))
            .await
            .unwrap();
        for charge in self.lease.charges(self.t.db()).await.unwrap() {
            if charge.status != ChargeStatus::Invoiced || Some(charge.due_date) == left_draft {
                continue;
            }
            self.issue_and_pay(charge.invoice_id.unwrap(), charge.amount)
                .await;
        }
    }

    async fn issue_and_pay(&self, invoice: i64, amount: i64) {
        let issued = self
            .t
            .post(&self.clerk, &format!("/invoices/{invoice}/issue"), &[])
            .await;
        assert!(
            issued.status.is_success() || issued.status.is_redirection(),
            "{}",
            issued.status
        );
        invoicer(self.t.state())
            .allocate(
                self.t.state(),
                InvoiceId(invoice),
                Subject::new("payments.payment", invoice),
                amount,
            )
            .await
            .unwrap();
    }

    async fn invoice_of(&self, due: NaiveDate) -> i64 {
        self.lease
            .charges(self.t.db())
            .await
            .unwrap()
            .iter()
            .find(|c| c.due_date == due)
            .and_then(|c| c.invoice_id)
            .expect("the invoice")
    }

    async fn notes_of(&self, invoice: i64) -> usize {
        invoicer(self.t.state())
            .credit_notes_for(self.t.db(), InvoiceId(invoice))
            .await
            .unwrap()
            .len()
    }

    /// The tenant's notice, the move-out inspection and the keys.
    async fn ready(&self) {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/notice", self.lease.id),
                &[
                    ("date", "2027-05-15"),
                    ("party", "tenant"),
                    ("reason", "early_termination"),
                    ("planned_move_out", "2027-06-20"),
                ],
            )
            .await
            .assert_ok();
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/move-out-inspection", self.lease.id),
                &[
                    ("done_on", "2027-06-19"),
                    ("keys_returned_on", "2027-06-20"),
                    ("defects", "no"),
                ],
            )
            .await
            .assert_ok();
    }

    async fn terminate(&self) -> renox::testing::TestResponse {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/terminate", self.lease.id),
                &[("move_out", "2027-06-20"), ("credit", "refund")],
            )
            .await
    }

    fn returned(&self) -> usize {
        self.t
            .cheques
            .all()
            .iter()
            .filter(|c| c.status == ChequeStatus::Returned)
            .count()
    }

    /// Nothing of the termination happened.
    async fn nothing_changed(&self) {
        let lease = self.t.reload(&self.lease).await;
        assert_eq!(lease.status, LeaseStatus::NoticeGiven);
        assert!(lease.actual_move_out.is_none());
        assert_eq!(lease.termination_credit, 0);
        assert_ne!(self.t.unit_now(&self.unit).await.status, UnitStatus::Vacant);
        assert!(self.t.cheques.returns().is_empty());
        assert_eq!(self.returned(), 0);
        assert!(
            self.t
                .cheques
                .all()
                .iter()
                .all(|c| c.status == ChequeStatus::Received)
        );
        assert!(self.t.deposits.is_held(&self.lease.agreement()));
        assert!(self.t.deposits.started().is_empty());
        // No "finish by hand" task, no event.
        assert!(
            self.t
                .tasks
                .with_key("property.terminate_problems")
                .is_empty(),
            "{:?}",
            self.t.tasks.open()
        );
        assert!(self.t.terminated.all().is_empty());
    }
}

/// Done when 1: with 2 invoices to credit and 5 cheques to return,
/// `terminate` commits the lease, the unit, both credit notes, the 5 returns
/// and the deposit settlement together.
#[renox::test]
async fn bl_prp_33_terminate_commits_every_money_effect_together() {
    let s = scene().await;
    s.invoices(None).await;
    s.ready().await;

    s.terminate().await.assert_ok();

    let lease = s.t.reload(&s.lease).await;
    assert_eq!(lease.status, LeaseStatus::Terminated);
    assert_eq!(s.t.unit_now(&s.unit).await.status, UnitStatus::Vacant);
    let june = s.invoice_of(d(6, 1)).await;
    let july = s.invoice_of(d(7, 1)).await;
    assert_eq!(s.notes_of(june).await, 1);
    assert_eq!(s.notes_of(july).await, 1);
    // 10 of 30 days of June, and July in full.
    assert_eq!(lease.termination_credit, aed(3_000) + aed(9_000));
    assert_eq!(s.returned(), 5);
    assert_eq!(s.t.cheques.returns().len(), 1, "against one receipt");
    assert_eq!(s.t.deposits.started(), vec![s.lease.agreement()]);
    assert!(s.t.tasks.with_key("property.terminate_problems").is_empty());
    assert_eq!(s.t.terminated.all().len(), 1);
}

/// Done when 2 and 3: when crediting the second invoice would fail (July was
/// credited in full already, so its rent line has nothing left), nothing
/// changes: the lease is `notice_given`, June has no credit note, the cheques
/// are `received`, the deposit is held and the user sees the reason; no task
/// is created and no `LeaseTerminated` is emitted.
#[renox::test]
async fn bl_prp_33_a_credit_that_would_fail_refuses_terminate_and_changes_nothing() {
    let s = scene().await;
    s.invoices(None).await;
    let june = s.invoice_of(d(6, 1)).await;
    let july = s.invoice_of(d(7, 1)).await;
    invoicer(s.t.state())
        .credit_partly(
            s.t.state(),
            InvoiceId(july),
            vec![CreditLine::new(1, aed(9_000))],
            "Goodwill".to_owned(),
            None,
        )
        .await
        .unwrap();
    s.ready().await;

    let refused = s.terminate().await;

    refused.assert_status(422);
    assert!(
        refused.text().contains("Nothing was changed"),
        "{}",
        refused.text()
    );
    s.nothing_changed().await;
    assert_eq!(
        s.notes_of(june).await,
        0,
        "the first credit note does not exist"
    );
    assert_eq!(s.notes_of(july).await, 1, "only the earlier one");
}

/// Done when 4: a draft that straddles the move-out date refuses `terminate`
/// with nothing changed; once the cause is fixed (the draft is issued),
/// `terminate` succeeds once.
#[renox::test]
async fn bl_prp_33_after_the_cause_is_fixed_terminate_succeeds_once() {
    let s = scene().await;
    s.invoices(Some(d(6, 1))).await;
    s.ready().await;

    let refused = s.terminate().await;
    refused.assert_status(422);
    s.nothing_changed().await;

    let june = s.invoice_of(d(6, 1)).await;
    s.issue_and_pay(june, aed(9_000)).await;
    s.terminate().await.assert_ok();

    let lease = s.t.reload(&s.lease).await;
    assert_eq!(lease.status, LeaseStatus::Terminated);
    assert_eq!(s.notes_of(june).await, 1);
    assert_eq!(s.notes_of(s.invoice_of(d(7, 1)).await).await, 1);
    assert_eq!(s.returned(), 5);
    assert_eq!(s.t.terminated.all().len(), 1);
    // A second run is refused (the lease is terminated) and adds nothing.
    assert!(!s.terminate().await.status.is_success());
    assert_eq!(s.notes_of(june).await, 1);
    assert_eq!(s.t.terminated.all().len(), 1);
}
