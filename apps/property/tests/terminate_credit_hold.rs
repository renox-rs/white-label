//! BL-PRP-32 (#477) with the real wl-invoicing and wl-payments: when a deposit
//! is held, the termination credit (what the pro rata credit note of a paid
//! invoice leaves) is held for the deposit settlement and covers the
//! deductions before the deposit; only the rest is refunded or kept as the
//! property manager chose at `terminate`.
//!
//! The lease runs 2027-01-01 to 2027-12-31 at AED 9,000 a month; July is paid,
//! so leaving on 2027-07-20 credits 11 of 31 days: AED 3,193.55. June is paid
//! only in part, which leaves unpaid rent for the deductions. (Done when 4, the
//! credit with no deposit held, is in `terminate_credit.rs`.)

mod support;

use property::deposit_forfeit::{DAMAGES, DepositSettlement};
use property::{ChargeStatus, CreditChoice, Lease, LeaseStatus, NewLease, charge_type};
use renox::chrono::{Duration as Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::deposits::{APPLIED_TO_DEDUCTIONS, DisputeDeductions, deposit_deductions};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{CreditNoteStatus, InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};
use wl_kernel::{LegalEntity, VatCategory};
use wl_payments::deposit::DepositStatus;
use wl_payments::{Deposit, Refund};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, Real, TENANT, aed, boot_with, d};

struct Scene {
    t: Booted,
    agent: User,
    manager: User,
    accountant: User,
    finance: User,
    teller: User,
    clerk: User,
    unit: property::Unit,
}

async fn scene() -> Scene {
    let t = boot_with(Real::MONEY).await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let finance = t.user_with("Fiona", "finance_manager").await;
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
        agent,
        manager,
        accountant,
        finance,
        teller,
        clerk,
        unit,
    }
}

impl Scene {
    /// An `active` lease 2027-01-01 to 2027-12-31 at AED 9,000 a month, with a
    /// deposit of AED 10,000 held by the real wl-payments.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(108_000), 12);
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.hold_real_deposit(&lease, aed(10_000)).await;
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.reload(&lease).await
    }

    /// The invoices due by July are drafted, issued and paid by transfer, all
    /// in full except June, of which only `june_paid` fils are paid.
    async fn invoice_and_pay(&self, lease: &Lease, june_paid: i64) {
        property::invoicing::run(self.t.state(), d(7, 1))
            .await
            .unwrap();
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
                .post(&self.clerk, &format!("/invoices/{invoice}/issue"), &[])
                .await;
            assert!(
                issued.status.is_success() || issued.status.is_redirection(),
                "{}",
                issued.status
            );
            let amount = if charge.due_date == d(6, 1) {
                june_paid
            } else {
                charge.amount
            };
            let mut request =
                RecordPayment::new(TENANT, entity.id, PaymentMethod::Transfer, amount);
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
                    amount,
                )
                .await
                .expect("the payment is allocated");
        }
    }

    /// The tenant's notice, the move-out inspection and the keys; then
    /// `terminate` on 2027-07-20 with the credit `choice`.
    async fn terminate(&self, lease: &Lease, choice: &str) -> Lease {
        for (path, form) in [
            (
                "notice",
                vec![
                    ("date", "2027-06-15"),
                    ("party", "tenant"),
                    ("reason", "end_of_term"),
                    ("planned_move_out", "2027-07-20"),
                ],
            ),
            (
                "move-out-inspection",
                vec![
                    ("done_on", "2027-07-19"),
                    ("keys_returned_on", "2027-07-20"),
                    ("defects", "no"),
                ],
            ),
            (
                "terminate",
                vec![("move_out", "2027-07-20"), ("credit", choice)],
            ),
        ] {
            self.t
                .post(
                    &self.manager,
                    &format!("/property/leases/{}/{path}", lease.id),
                    &form,
                )
                .await
                .assert_ok();
        }
        self.t.reload(lease).await
    }

    /// The invoice of the charge due on `due`.
    async fn invoice_of(&self, lease: &Lease, due: NaiveDate) -> i64 {
        lease
            .charges(self.t.db())
            .await
            .unwrap()
            .iter()
            .find(|c| c.due_date == due)
            .and_then(|c| c.invoice_id)
            .expect("the invoice")
    }

    async fn balance(&self, invoice: i64) -> (InvoiceStatus, i64) {
        let found = invoicer(self.t.state())
            .find(self.t.db(), InvoiceId(invoice))
            .await
            .unwrap()
            .expect("the invoice");
        (found.status, found.balance)
    }

    /// The credit note of the July invoice: its status and what is left.
    async fn july_note(&self, lease: &Lease) -> (CreditNoteStatus, i64) {
        let july = self.invoice_of(lease, d(7, 1)).await;
        let notes = invoicer(self.t.state())
            .credit_notes_for(self.t.db(), InvoiceId(july))
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        (notes[0].status, notes[0].remaining)
    }

    async fn deposit(&self, lease: &Lease) -> Deposit {
        Deposit::query()
            .where_(Deposit::SOURCE_ID.eq(lease.agreement().subject_id))
            .first(self.t.db())
            .await
            .unwrap()
            .expect("the deposit")
    }

    /// The teller publishes `lines` (`description | amount | evidence` per
    /// row), as a forfeiture applied to the deductions when `forfeiture`.
    async fn publish(&self, lease: &Lease, lines: &str, forfeiture: bool) {
        let deposit = self.deposit(lease).await;
        let mut form = vec![("lines", lines)];
        if forfeiture {
            form.push(("forfeiture_reason", APPLIED_TO_DEDUCTIONS));
        }
        self.t
            .post(
                &self.teller,
                &format!("/deposits/{}/deductions", deposit.id),
                &form,
            )
            .await
            .assert_ok();
    }

    /// The accountant refunds the deposit by transfer after the window.
    async fn refund_deposit(&self, lease: &Lease) {
        let deposit = self.deposit(lease).await;
        let response = self
            .t
            .at(
                today() + Days::days(15),
                self.t.post(
                    &self.accountant,
                    &format!("/deposits/{}/refund", deposit.id),
                    &[("method", "transfer")],
                ),
            )
            .await;
        assert!(
            response.status.is_success() || response.status.is_redirection(),
            "{}: {}",
            response.status,
            response.text()
        );
    }

    /// The accountant forfeits the deposit applied to the deductions after the
    /// window, approved by the manager and by finance.
    async fn forfeit(&self, lease: &Lease) -> renox::testing::TestResponse {
        let approver = self.manager.id.to_string();
        let second = self.finance.id.to_string();
        let path = format!("/property/leases/{}/apply-deposit", lease.id);
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

    async fn refunds(&self) -> Vec<Refund> {
        Refund::query()
            .where_(Refund::PARTY_ID.eq(TENANT.0))
            .order_by(Refund::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// `terminate` with a deposit held: the credit of AED 3,193.55 is held for the
/// settlement, nothing is refunded yet and the credit note keeps it; the lease
/// page says so.
#[renox::test]
async fn bl_prp_32_the_credit_is_held_for_the_settlement_when_a_deposit_is_held() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, aed(7_000)).await;

    let after = s.terminate(&lease, "refund").await;

    assert_eq!(after.status, LeaseStatus::Terminated);
    assert_eq!(after.termination_credit, 319_355);
    assert_eq!(after.credit_held, 319_355);
    assert!(after.credit_settled_on.is_none());
    assert_eq!(
        s.july_note(&after).await,
        (CreditNoteStatus::Issued, 319_355)
    );
    assert!(s.refunds().await.is_empty(), "nothing is refunded yet");
    assert_eq!(
        s.deposit(&lease).await.status,
        DepositStatus::SettlementPending
    );
    s.t.app.acting_as(&s.manager);
    s.t.app
        .get(&format!("/property/leases/{}", lease.id))
        .await
        .assert_ok()
        .assert_see("Termination credit held for the settlement")
        .assert_see("AED 3,193.55");
}

/// Done when 1: a credit of AED 3,193.55, a deposit of AED 10,000 and
/// deductions of AED 2,000 (the unpaid rest of June): at `refund` the credit
/// covers the AED 2,000, AED 1,193.55 is refunded (choice `refund`) and the
/// full deposit of AED 10,000 is refunded.
#[renox::test]
async fn bl_prp_32_at_refund_the_credit_covers_the_deductions_and_the_rest_is_refunded() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, aed(7_000)).await;
    let lease = s.terminate(&lease, "refund").await;
    let june = s.invoice_of(&lease, d(6, 1)).await;
    assert_eq!(
        s.balance(june).await,
        (InvoiceStatus::PartiallyPaid, aed(2_000))
    );

    // No damage lines: the deductions are the unpaid rent.
    s.publish(&lease, "", false).await;
    s.refund_deposit(&lease).await;

    assert_eq!(s.deposit(&lease).await.status, DepositStatus::Refunded);
    assert_eq!(s.balance(june).await, (InvoiceStatus::Paid, 0));
    let after = s.t.reload(&lease).await;
    assert_eq!(after.credit_to_deductions, aed(2_000));
    assert_eq!(after.credit_refunded, 119_355);
    assert_eq!(after.credit_kept, 0);
    assert_eq!(s.july_note(&after).await, (CreditNoteStatus::Applied, 0));
    let refunds = s.refunds().await;
    let mut amounts: Vec<(String, i64)> = refunds
        .iter()
        .map(|r| (r.source_kind.clone(), r.amount))
        .collect();
    amounts.sort();
    assert_eq!(
        amounts,
        vec![
            ("credit_note".to_owned(), 119_355),
            ("deposit".to_owned(), aed(10_000)),
        ],
        "the credit left and the full deposit"
    );
    // Settled once: a second run does nothing.
    let again = property::termination_credit::settle_held(s.t.state(), lease.root_id, None)
        .await
        .unwrap();
    assert!(again.is_none());
    assert_eq!(s.refunds().await.len(), 2);
}

/// Done when 2: with the choice `keep`, the AED 1,193.55 stays on the tenant's
/// account.
#[renox::test]
async fn bl_prp_32_with_the_keep_choice_the_rest_stays_on_account() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, aed(7_000)).await;
    let lease = s.terminate(&lease, "keep").await;
    assert_eq!(lease.credit_choice, CreditChoice::Keep);

    s.publish(&lease, "", false).await;
    s.refund_deposit(&lease).await;

    let after = s.t.reload(&lease).await;
    assert_eq!(after.credit_to_deductions, aed(2_000));
    assert_eq!(after.credit_kept, 119_355);
    assert_eq!(after.credit_refunded, 0);
    assert_eq!(
        s.july_note(&after).await,
        (CreditNoteStatus::Issued, 119_355)
    );
    assert_eq!(
        invoicer(s.t.state())
            .open_credit(s.t.db(), TENANT, "AED")
            .await
            .unwrap(),
        119_355,
        "the tenant's credit"
    );
    let refunds = s.refunds().await;
    assert_eq!(refunds.len(), 1, "only the deposit");
    assert_eq!(refunds[0].amount, aed(10_000));
}

/// Done when 3: deductions of AED 14,000 (rent AED 4,500 + damages AED 9,500):
/// the credit pays AED 3,193.55 of the rent invoice, the deposit covers the
/// rest of the rent and AED 8,693.55 of damages, and a damages invoice of AED
/// 806.45 is raised.
#[renox::test]
async fn bl_prp_32_the_credit_pays_the_rent_before_the_deposit_and_less_damages_are_invoiced() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, aed(4_500)).await;
    let lease = s.terminate(&lease, "refund").await;
    let june = s.invoice_of(&lease, d(6, 1)).await;

    s.publish(
        &lease,
        "Repainting | 6000.00 | photos-1\nChiller coil | 3500.00 | photos-2",
        true,
    )
    .await;
    s.forfeit(&lease).await.assert_ok();

    assert_eq!(s.deposit(&lease).await.status, DepositStatus::Forfeited);
    assert_eq!(s.balance(june).await, (InvoiceStatus::Paid, 0));
    let settlement = DepositSettlement::of_root(s.t.db(), lease.root_id)
        .await
        .unwrap()
        .expect("a settlement statement");
    assert_eq!(settlement.credit_applied, 319_355);
    assert_eq!(settlement.rent_due, aed(4_500));
    assert_eq!(settlement.damages, aed(9_500));
    assert_eq!(settlement.to_rent, aed(4_500) - 319_355);
    assert_eq!(settlement.to_damages, 869_355);
    assert_eq!(settlement.excess, 80_645);
    let after = s.t.reload(&lease).await;
    assert_eq!(after.credit_to_deductions, 319_355);
    assert_eq!(after.credit_refunded, 0);
    assert_eq!(after.credit_kept, 0);
    assert_eq!(s.july_note(&after).await, (CreditNoteStatus::Applied, 0));
    assert!(s.refunds().await.is_empty(), "nothing is paid back");
}

/// Done when 5: while a deduction line is disputed nothing is applied: the
/// forfeiture is refused and the held credit stays on the credit note.
#[renox::test]
async fn bl_prp_32_nothing_is_applied_while_a_deduction_line_is_disputed() {
    let s = scene().await;
    let lease = s.leased().await;
    s.invoice_and_pay(&lease, aed(4_500)).await;
    let lease = s.terminate(&lease, "refund").await;
    s.publish(&lease, "Repainting | 9500.00 | photos-1", true)
        .await;
    let published = deposit_deductions(s.t.state())
        .published_for(s.t.state(), TENANT)
        .await
        .unwrap()
        .remove(0);
    deposit_deductions(s.t.state())
        .dispute(
            s.t.state(),
            TENANT,
            DisputeDeductions {
                deposit: published.deposit,
                version: published.version,
                lines: vec![published.lines[0].id],
                comment: "The walls were already marked".to_owned(),
            },
        )
        .await
        .unwrap();

    s.forfeit(&lease).await.assert_status(422);
    let held = property::termination_credit::settle_held(s.t.state(), lease.root_id, None)
        .await
        .unwrap();
    assert!(held.is_none());

    let after = s.t.reload(&lease).await;
    assert_eq!(after.credit_held, 319_355);
    assert!(after.credit_settled_on.is_none());
    assert_eq!(after.credit_to_deductions, 0);
    assert_eq!(
        s.july_note(&after).await,
        (CreditNoteStatus::Issued, 319_355)
    );
    let june = s.invoice_of(&lease, d(6, 1)).await;
    assert_eq!(
        s.balance(june).await,
        (InvoiceStatus::PartiallyPaid, aed(4_500))
    );
    assert!(s.refunds().await.is_empty());
}
