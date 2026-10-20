//! A cheque given for a lease charge clears, with the real money modules
//! (wl-payments keeps the cheque and runs `clear`, wl-invoicing makes the
//! invoices). The cheque stays linked to the charge
//! (`property.lease_charge:<id>`); once the charge is invoiced, the payment
//! `clear` records goes to the invoice that bills it (wl-payments §4
//! *Cheque* `clear`: "allocated to the linked document"; 01 Property §4
//! step 11: "Payments are matched to invoices"); so does the honoured part of
//! a bounce. Checks of BL-PAY-09 (#102) "a payment of AED 6,000.00 is
//! allocated to the rent invoice" and BL-PAY-10 (#103) "`clear` ... records a
//! payment for the open amount".

mod support;

use property::{ChargeStatus, Lease, LeaseCharge, NewLease};
use renox::prelude::*;
use wl_invoicing::WlInvoicing;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::payments::PaymentSource;
use wl_payments::allocation::Allocation;
use wl_payments::credit::credit_balance;
use wl_payments::{Cheque, Payment, WlPayments, permissions as pay};

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot_money_with, day};

struct Scene {
    t: Booted,
    lease: Lease,
    staff: User,
}

/// An active lease of a residential unit (term from 300 days ago, AED 100,000
/// a year in 4 charges of AED 25,000, no deposit) and a cheque clerk.
async fn scene() -> Scene {
    let t = boot_money_with(
        WlInvoicing::new().auto_issue_source("property.lease"),
        WlPayments::new(),
    )
    .await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    let new = NewLease::new(unit.id, TENANT, day(-300), day(64), aed(100_000), 4);
    let lease = property::leases::create_draft(t.state(), None, new)
        .await
        .unwrap();
    t.submit(&agent, &lease).await.assert_ok();
    t.activate(&manager, &lease).await.assert_ok();
    let lease = t.reload(&lease).await;
    let staff = t
        .user_with_grants(
            "Amal",
            &[pay::VIEW, pay::CHEQUES_VIEW, pay::CHEQUES_DEPOSIT],
        )
        .await;
    Scene { t, lease, staff }
}

impl Scene {
    async fn charges(&self) -> Vec<LeaseCharge> {
        LeaseCharge::of_lease(self.t.db(), self.lease.id)
            .await
            .unwrap()
    }

    /// A cheque of AED 25,000 dated today for the last rent charge,
    /// registered in wl-payments with the lease.
    async fn cheque_for_last_charge(&self, number: &str) -> (Cheque, LeaseCharge) {
        let charge = self.charges().await[3].clone();
        assert_eq!(charge.status, ChargeStatus::Scheduled);
        let registered = property::cheques::register_cheques(
            self.t.state(),
            None,
            &self.lease,
            "ENBD",
            vec![property::cheques::ChequeEntry::new(
                number,
                today(),
                aed(25_000),
                Some(charge.id),
            )],
        )
        .await
        .unwrap();
        let id = registered[0].id.0;
        (Cheque::find_or_404(self.t.db(), id).await.unwrap(), charge)
    }

    /// Runs the daily invoicing of today and issues its drafts, as the
    /// accountant does; returns the charges, every one invoiced.
    async fn invoice_all(&self) -> Vec<LeaseCharge> {
        property::invoicing::run(self.t.state(), today())
            .await
            .unwrap();
        let charges = self.charges().await;
        assert!(charges.iter().all(|c| c.status == ChargeStatus::Invoiced));
        for c in &charges {
            let id = InvoiceId(c.invoice_id.expect("the invoice"));
            let found = invoicer(self.t.state())
                .find(self.t.db(), id)
                .await
                .unwrap();
            if found.is_some_and(|i| i.status == InvoiceStatus::Draft) {
                invoicer(self.t.state())
                    .issue(self.t.state(), id, None)
                    .await
                    .unwrap();
            }
        }
        charges
    }

    /// Deposits and clears `cheque`; returns the payment `clear` recorded.
    async fn clear(&self, cheque: &Cheque) -> Payment {
        let path = format!("/cheques/{}", cheque.id);
        self.t
            .post(&self.staff, &format!("{path}/deposit"), &[])
            .await
            .assert_ok();
        self.t
            .post(&self.staff, &format!("{path}/clear"), &[])
            .await
            .assert_ok();
        let payments = Payment::query()
            .where_(Payment::PARTY_ID.eq(TENANT.0))
            .get(self.t.db())
            .await
            .unwrap();
        let ours: Vec<Payment> = payments
            .into_iter()
            .filter(|p| p.source == PaymentSource::Cheque)
            .collect();
        assert_eq!(ours.len(), 1, "one payment for the cheque");
        assert_eq!(ours[0].amount, aed(25_000));
        ours[0].clone()
    }
}

/// The cheque names the last charge; the daily run invoices every charge due
/// (the last one included) and the invoices are issued before the cheque
/// clears. `clear` allocates the AED 25,000 to the invoice of that charge, which is `paid`; the older
/// invoices keep their balance and the tenant has no credit.
#[renox::test]
async fn a_cheque_for_an_invoiced_charge_pays_its_invoice() {
    let s = scene().await;
    let (cheque, charge) = s.cheque_for_last_charge("2001").await;
    let charges = s.invoice_all().await;
    let charge = charges.iter().find(|c| c.id == charge.id).unwrap();
    let invoice = InvoiceId(charge.invoice_id.expect("the invoice"));
    let before = invoicer(s.t.state())
        .find(s.t.db(), invoice)
        .await
        .unwrap()
        .expect("the invoice");
    assert!(before.status.is_open(), "issued: {:?}", before.status);
    assert_eq!(before.balance, aed(25_000));

    let payment = s.clear(&cheque).await;

    let allocations = Allocation::active_of(s.t.db(), payment.id).await.unwrap();
    assert_eq!(allocations.len(), 1, "{allocations:?}");
    assert_eq!(allocations[0].target(), invoice.subject());
    assert_eq!(allocations[0].amount, aed(25_000));
    let after = invoicer(s.t.state())
        .find(s.t.db(), invoice)
        .await
        .unwrap()
        .expect("the invoice");
    assert_eq!(after.status, InvoiceStatus::Paid);
    assert_eq!(after.balance, 0);
    for other in charges.iter().filter(|c| c.id != charge.id) {
        let found = invoicer(s.t.state())
            .find(s.t.db(), InvoiceId(other.invoice_id.unwrap()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            found.balance,
            aed(25_000),
            "only the cheque's charge is paid"
        );
    }
    assert_eq!(credit_balance(s.t.db(), TENANT, "AED").await.unwrap(), 0);
}

/// The charge is not invoiced yet when the cheque clears: there is no
/// invoice to pay, so the AED 25,000 stays on the payment as the tenant's
/// credit (wl-payments: what is left unallocated stays the party's credit),
/// and the charge stays `scheduled`.
#[renox::test]
async fn a_cheque_for_a_charge_not_yet_invoiced_stays_the_tenants_credit() {
    let s = scene().await;
    let (cheque, charge) = s.cheque_for_last_charge("2002").await;

    let payment = s.clear(&cheque).await;

    let allocations = Allocation::active_of(s.t.db(), payment.id).await.unwrap();
    assert!(allocations.is_empty(), "{allocations:?}");
    let charge = LeaseCharge::find_or_404(s.t.db(), charge.id).await.unwrap();
    assert_eq!(charge.status, ChargeStatus::Scheduled);
    assert_eq!(charge.invoice_id, None);
    assert_eq!(
        credit_balance(s.t.db(), TENANT, "AED").await.unwrap(),
        aed(25_000)
    );
}

/// #102 "a payment of AED 6,000.00 is allocated to the rent invoice": the
/// cheque for the invoiced last charge bounces with AED 6,000 honoured. The
/// honoured payment goes to the invoice of that charge (open AED 19,000), and
/// the tenant has no credit.
#[renox::test]
async fn the_honoured_part_of_a_bounce_pays_the_charges_invoice() {
    let s = scene().await;
    let (cheque, charge) = s.cheque_for_last_charge("2003").await;
    let charges = s.invoice_all().await;
    let charge = charges.iter().find(|c| c.id == charge.id).unwrap();
    let invoice = InvoiceId(charge.invoice_id.expect("the invoice"));

    let path = format!("/cheques/{}", cheque.id);
    s.t.post(&s.staff, &format!("{path}/deposit"), &[])
        .await
        .assert_ok();
    s.t.post(
        &s.staff,
        &format!("{path}/bounce"),
        &[
            ("reason", "insufficient_funds"),
            ("certificate", "CERT-2"),
            ("honoured", "6000.00"),
        ],
    )
    .await
    .assert_ok();

    let cheque = Cheque::find_or_404(s.t.db(), cheque.id).await.unwrap();
    assert_eq!(cheque.open_amount, aed(19_000));
    let payment = Payment::query()
        .where_(Payment::PARTY_ID.eq(TENANT.0))
        .get(s.t.db())
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.source == PaymentSource::Cheque)
        .expect("the honoured payment");
    assert_eq!(payment.amount, aed(6_000));
    let allocations = Allocation::active_of(s.t.db(), payment.id).await.unwrap();
    assert_eq!(allocations.len(), 1, "{allocations:?}");
    assert_eq!(allocations[0].target(), invoice.subject());
    let found = invoicer(s.t.state())
        .find(s.t.db(), invoice)
        .await
        .unwrap()
        .expect("the invoice");
    assert_eq!(found.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(found.balance, aed(19_000));
    assert_eq!(credit_balance(s.t.db(), TENANT, "AED").await.unwrap(), 0);
}
