//! BL-PRP-11 (#261) with the real wl-invoicing and wl-payments: the credit
//! note of the pro rata credit of a paid invoice is the tenant's credit, and
//! with no deposit held `terminate` either asks it back through the Refund
//! flow or keeps it, as the property manager chose (BL-PRP-32 Done when 4: as
//! before; with a deposit held the credit waits for the settlement, see
//! `terminate_credit_hold.rs`). A step that fails is left to the accountant.

mod support;

use property::termination::{self, Settlement};
use property::{ChargeStatus, CreditChoice, Lease, LeaseCharge, LeaseStatus, NewLease};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::CreditNote;
use wl_kernel::LegalEntity;
use wl_kernel::deposits::deposits;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::{
    AllocationTarget, PaymentMethod, RecordPayment, RefundSource, payment_recorder,
};
use wl_payments::Refund;
use wl_payments::refund::RefundStatus;

use support::{AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with, d};

struct Scene {
    t: Booted,
    agent: User,
    manager: User,
    unit: property::Unit,
}

async fn scene(real: Real) -> Scene {
    let t = boot_with(real).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        agent,
        manager,
        unit,
    }
}

impl Scene {
    /// An `active` lease 2027-01-01 to 2027-12-31 at AED 9,000 a month.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(108_000), 12);
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.reload(&lease).await
    }

    /// The daily run drafts the invoices due by `day`; a clerk issues them
    /// and the tenant pays each one by a transfer, allocated by the real
    /// payments (or straight on the invoice when only invoicing is real).
    async fn invoice_and_pay(&self, lease: &Lease, day: NaiveDate, real_payments: bool) {
        property::invoicing::run(self.t.state(), day).await.unwrap();
        renox::auth::permissions::define_role(
            self.t.db(),
            "invoice_clerk",
            &[
                wl_invoicing::permissions::VIEW,
                wl_invoicing::permissions::ISSUE,
            ],
        )
        .await
        .unwrap();
        let clerk = self.t.user_with("Clara", "invoice_clerk").await;
        let entity = LegalEntity::query()
            .where_(LegalEntity::CODE.eq("PGP"))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the default entity");
        let mut charges = lease.charges(self.t.db()).await.unwrap();
        charges.sort_by_key(|c| (c.due_date, c.id));
        for charge in charges
            .iter()
            .filter(|c| c.status == ChargeStatus::Invoiced)
        {
            let invoice = charge.invoice_id.expect("an invoice");
            let issued = self
                .t
                .post(&clerk, &format!("/invoices/{invoice}/issue"), &[])
                .await;
            assert!(
                issued.status.is_success() || issued.status.is_redirection(),
                "{}: {}",
                issued.status,
                issued.text()
            );
            if !real_payments {
                invoicer(self.t.state())
                    .allocate(
                        self.t.state(),
                        InvoiceId(invoice),
                        wl_kernel::Subject::new("payments.payment", invoice),
                        charge.amount,
                    )
                    .await
                    .expect("the invoice is paid");
                continue;
            }
            let mut request =
                RecordPayment::new(TENANT, entity.id, PaymentMethod::Transfer, charge.amount);
            request.reference = Some(format!("TRF-{invoice}"));
            let payment = payment_recorder(self.t.state())
                .record(self.t.state(), request, None)
                .await
                .expect("the payment is recorded");
            payment_recorder(self.t.state())
                .allocate(
                    self.t.state(),
                    payment.id,
                    AllocationTarget::Invoice(InvoiceId(invoice)),
                    charge.amount,
                )
                .await
                .expect("the payment is allocated");
        }
    }

    /// The tenant's notice (end of term, leaving 2027-07-20), the move-out
    /// inspection and the keys; then `terminate` with the credit `choice`.
    async fn notice_and_terminate(&self, lease: &Lease, choice: &str) {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/notice", lease.id),
                &[
                    ("date", "2027-06-15"),
                    ("party", "tenant"),
                    ("reason", "end_of_term"),
                    ("planned_move_out", "2027-07-20"),
                ],
            )
            .await
            .assert_ok();
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/move-out-inspection", lease.id),
                &[
                    ("done_on", "2027-07-19"),
                    ("keys_returned_on", "2027-07-20"),
                    ("defects", "no"),
                ],
            )
            .await
            .assert_ok();
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/terminate", lease.id),
                &[("move_out", "2027-07-20"), ("credit", choice)],
            )
            .await
            .assert_ok();
    }

    /// The credit notes of the July invoice.
    async fn july_note(&self, lease: &Lease) -> CreditNote {
        let charges = lease.charges(self.t.db()).await.unwrap();
        let july = charges.iter().find(|c| c.due_date == d(7, 1)).unwrap();
        let notes = invoicer(self.t.state())
            .credit_notes_for(self.t.db(), InvoiceId(july.invoice_id.unwrap()))
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        CreditNote::find_or_404(self.t.db(), notes[0].id.0)
            .await
            .unwrap()
    }
}

/// AC1, the refund choice: the July invoice was paid, so the credit note of
/// AED 3,193.55 (11 of 31 days) is the tenant's credit. No deposit is held
/// (BL-PRP-32 Done when 4), so with the choice to refund it, the credit note is
/// the source of a refund of that amount at `terminate`, which waits for
/// another user's approval; the credit note is used up. The portal event says
/// what was credited.
#[renox::test]
async fn bl_prp_11_the_credit_that_is_left_is_asked_back_when_the_manager_chose_to_refund_it() {
    let s = scene(Real::MONEY).await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, d(7, 1), true).await;

    s.notice_and_terminate(&lease, "refund").await;

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Terminated);
    assert_eq!(after.credit_choice, CreditChoice::Refund);
    assert_eq!(after.termination_credit, 319_355);

    // The July invoice stays paid; its credit note was refunded.
    let note = s.july_note(&after).await;
    assert_eq!(note.gross_total, 319_355);
    assert_eq!(note.remaining, 0, "the refund took the whole credit");
    assert_eq!(note.status, CreditNoteStatus::Applied);
    let refunds = Refund::query()
        .where_(Refund::PARTY_ID.eq(TENANT.0))
        .get(s.t.db())
        .await
        .unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 319_355);
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(refunds[0].source_kind, "credit_note");
    assert_eq!(
        RefundSource::CreditNote(wl_kernel::Subject::new(
            &refunds[0].source_type,
            refunds[0].source_id
        )),
        RefundSource::CreditNote(wl_kernel::invoicing::CreditNoteId(note.id).subject())
    );
    assert!(refunds[0].reason.contains(&after.number_with_version()));
    assert!(
        s.t.tasks.with_key("property.terminate_problems").is_empty(),
        "{:?}",
        s.t.tasks.open()
    );

    // No deposit was held: nothing waits for a settlement.
    assert_eq!(after.credit_held, 0);
    assert!(
        deposits(s.t.state())
            .held_for(s.t.db(), &lease.agreement())
            .await
            .unwrap()
            .is_none()
    );

    // The portal event carries the move-out and the credit.
    let events = s.t.terminated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].lease, lease.id);
    assert_eq!(events[0].root, lease.root_id);
    assert_eq!(events[0].unit, s.unit.id);
    assert_eq!(events[0].tenant, TENANT);
    assert_eq!(events[0].move_out, d(7, 20));
    assert_eq!(events[0].rent_due_until, None);
    assert_eq!(events[0].credited, 319_355);

    // A repeat asks nothing more.
    let again = termination::settle(s.t.state(), lease.id, None)
        .await
        .unwrap();
    assert_eq!(again, Settlement::default());
    assert_eq!(
        Refund::query()
            .where_(Refund::PARTY_ID.eq(TENANT.0))
            .count(s.t.db())
            .await
            .unwrap(),
        1
    );
}

/// AC1, the keep choice: with no deposit held the credit stays on the credit
/// note for the tenant to use, and no refund is asked.
#[renox::test]
async fn bl_prp_11_the_credit_is_kept_for_the_tenant_when_the_manager_chose_to_keep_it() {
    let s = scene(Real::MONEY).await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, d(7, 1), true).await;

    s.notice_and_terminate(&lease, "keep").await;

    let after = s.t.reload(&lease).await;
    assert_eq!(after.credit_choice, CreditChoice::Keep);
    let note = s.july_note(&after).await;
    assert_eq!(note.remaining, 319_355);
    assert_eq!(note.status, CreditNoteStatus::Issued);
    assert_eq!(
        invoicer(s.t.state())
            .open_credit(s.t.db(), TENANT, "AED")
            .await
            .unwrap(),
        319_355,
        "the tenant's credit"
    );
    assert_eq!(Refund::query().count(s.t.db()).await.unwrap(), 0);
    assert!(
        s.t.tasks.with_key("property.terminate_problems").is_empty(),
        "{:?}",
        s.t.tasks.open()
    );
    assert_eq!(s.t.terminated.all()[0].credited, 319_355);
}

/// A July invoice that is not paid is reduced by its credit note, so no credit
/// is left to refund.
#[renox::test]
async fn bl_prp_11_an_unpaid_invoice_leaves_no_credit_to_refund() {
    let s = scene(Real::MONEY).await;
    let lease = s.leased().await;
    s.t.hold_real_deposit(&lease, aed(5_000)).await;
    s.invoice_and_pay(&lease, d(6, 1), true).await;
    // July is invoiced and issued but not paid.
    property::invoicing::run(s.t.state(), d(7, 1))
        .await
        .unwrap();
    let july = lease
        .charges(s.t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|c: &LeaseCharge| c.due_date == d(7, 1))
        .unwrap();
    let invoice = july.invoice_id.unwrap();
    renox::auth::permissions::define_role(
        s.t.db(),
        "invoice_issuer",
        &[
            wl_invoicing::permissions::VIEW,
            wl_invoicing::permissions::ISSUE,
        ],
    )
    .await
    .unwrap();
    let issuer = s.t.user_with("Ivan", "invoice_issuer").await;
    s.t.post(&issuer, &format!("/invoices/{invoice}/issue"), &[])
        .await;

    s.notice_and_terminate(&lease, "refund").await;

    let summary = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(invoice))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(summary.balance, 900_000 - 319_355);
    assert_eq!(Refund::query().count(s.t.db()).await.unwrap(), 0);
    assert!(
        s.t.tasks.with_key("property.terminate_problems").is_empty(),
        "{:?}",
        s.t.tasks.open()
    );
}

/// A refund that payments refuses is a task for the accountant, listing what
/// to finish by hand; the credit stays on the credit note, the other steps ran
/// and the event is still emitted.
#[renox::test]
async fn bl_prp_11_a_refund_that_fails_is_left_to_the_accountant() {
    let s = scene(Real::INVOICING).await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, d(7, 1), false).await;
    s.t.refunds.refuse(true);

    s.notice_and_terminate(&lease, "refund").await;

    let after = s.t.reload(&lease).await;
    assert_eq!(after.status, LeaseStatus::Terminated);
    let note = s.july_note(&after).await;
    assert_eq!(note.remaining, 319_355, "nothing was taken from the credit");
    assert!(s.t.refunds.asked().is_empty());
    // One task for the accountant, naming the refund.
    let tasks = s.t.tasks.with_key("property.terminate_problems");
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    assert_eq!(
        tasks[0].key.as_deref(),
        Some(format!("property.terminate_problems:{}", lease.id).as_str())
    );
    assert!(
        tasks[0]
            .description
            .contains(&format!("refund of credit note {}", note.id)),
        "{:?}",
        tasks[0].description
    );
    // The rest of the termination went through.
    assert_eq!(s.t.terminated.all().len(), 1);
}
