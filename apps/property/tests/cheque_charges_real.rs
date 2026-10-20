//! The lease charges behind a cheque, with the real money modules (wl-payments
//! keeps the cheques and runs `bounce` and `write_off`, wl-invoicing makes the
//! invoices): 01 Property answers the kernel `BounceFees` and
//! `ChargeInvoicer` for the charges of its leases. Checks of BL-PAY-09 (#102)
//! "With a 01 fee charge type of AED 500.00, a charge of AED 500.00 + VAT
//! (standard default) is added" and BL-PAY-13 (#106) "Given a linked charge
//! not yet invoiced, the charge is invoiced first, then written off".

mod support;

use property::{ChargeStatus, Lease, LeaseCharge, NewLease};
use renox::prelude::*;
use wl_invoicing::WlInvoicing;
use wl_kernel::Subject;
use wl_kernel::VatCategory;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_payments::cheque_write_off::ChequeWriteOff;
use wl_payments::followup::FollowupKind;
use wl_payments::{Cheque, ChequeReturn, FollowupTask, WlPayments, permissions as pay};

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot_money_with, day};

/// The fee charge type the client sets.
const FEE_TYPE: &str = "returned_cheque_fee";

struct Scene {
    t: Booted,
    lease: Lease,
    staff: User,
}

/// An active lease of a residential unit (term from 300 days ago, AED 100,000
/// a year in 4 charges, no deposit) and a cheque clerk who may also ask for an
/// invoice write-off.
async fn scene(payments: WlPayments) -> Scene {
    let t = boot_money_with(
        WlInvoicing::new().auto_issue_source("property.lease"),
        payments,
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
            &[
                pay::VIEW,
                pay::CHEQUES_VIEW,
                pay::CHEQUES_DEPOSIT,
                pay::CHEQUES_MANAGE,
                wl_invoicing::permissions::WRITE_OFF,
            ],
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

    /// A cheque of AED 25,000 dated today for the last rent charge, registered
    /// in wl-payments with the lease, deposited and bounced.
    async fn bounced(&self, number: &str) -> (Cheque, LeaseCharge) {
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
        let path = format!("/cheques/{id}");
        self.t
            .post(&self.staff, &format!("{path}/deposit"), &[])
            .await
            .assert_ok();
        self.t
            .post(
                &self.staff,
                &format!("{path}/bounce"),
                &[
                    ("reason", "insufficient_funds"),
                    ("certificate", "CERT-1"),
                    ("honoured", ""),
                ],
            )
            .await
            .assert_ok();
        (Cheque::find_or_404(self.t.db(), id).await.unwrap(), charge)
    }
}

/// #102: with a fee of AED 500.00 set, the bounce adds a scheduled *returned
/// cheque fee* charge of AED 500.00 at standard VAT to the lease, due on the
/// bounce date; the accountant gets no task to raise it, and the daily run
/// invoices it at AED 525.00.
#[renox::test]
async fn a_bounce_adds_the_fee_charge_to_the_lease() {
    let s = scene(WlPayments::new().bounce_fee(FEE_TYPE, aed(500))).await;
    let (cheque, _) = s.bounced("1044").await;

    let fees: Vec<LeaseCharge> = s
        .charges()
        .await
        .into_iter()
        .filter(|c| c.charge_type == FEE_TYPE)
        .collect();
    assert_eq!(fees.len(), 1);
    let fee = &fees[0];
    assert_eq!(fee.amount, aed(500));
    assert_eq!(fee.vat_category, VatCategory::Standard);
    assert_eq!(fee.due_date, today());
    assert_eq!(fee.status, ChargeStatus::Scheduled);
    assert_eq!(
        fee.description,
        format!("Returned cheque fee · {} v1", s.lease.number())
    );

    let returns = ChequeReturn::of_cheque(s.t.db(), cheque.id).await.unwrap();
    assert_eq!(returns[0].fee_amount, aed(500));
    assert_eq!(returns[0].fee_charge(), Some(fee.subject()));
    let tasks = FollowupTask::of_cheque(s.t.db(), cheque.id).await.unwrap();
    assert!(tasks.iter().all(|t| t.kind != FollowupKind::RaiseBounceFee));

    property::invoicing::run(s.t.state(), today())
        .await
        .unwrap();
    let fee = LeaseCharge::find_or_404(s.t.db(), fee.id).await.unwrap();
    assert_eq!(fee.status, ChargeStatus::Invoiced);
    let invoice = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(fee.invoice_id.unwrap()))
        .await
        .unwrap()
        .expect("the fee invoice");
    assert_eq!(invoice.gross_total, aed(525));
    assert_eq!(invoice.party, TENANT);
}

/// Without a fee set the bounce adds no charge (#276: none until the client
/// sets one).
#[renox::test]
async fn no_fee_charge_until_the_client_sets_one() {
    let s = scene(WlPayments::new()).await;
    let before = s.charges().await.len();
    s.bounced("1045").await;
    assert_eq!(s.charges().await.len(), before);
}

/// #106: the bounced cheque pays a scheduled lease charge that is not invoiced
/// yet. Its `write_off` first invoices the charge through wl-invoicing (the
/// invoice is issued and the charge `invoiced`), moves the cheque's link to
/// the invoice, then asks for the invoice write-off.
#[renox::test]
async fn a_lease_charge_not_yet_invoiced_is_invoiced_first_then_written_off() {
    let s = scene(WlPayments::new()).await;
    let (cheque, charge) = s.bounced("1046").await;

    s.t.post(
        &s.staff,
        &format!("/cheques/{}/write_off", cheque.id),
        &[("reason", "untraceable")],
    )
    .await
    .assert_ok();

    let charge = LeaseCharge::find_or_404(s.t.db(), charge.id).await.unwrap();
    assert_eq!(charge.status, ChargeStatus::Invoiced);
    let invoice = InvoiceId(charge.invoice_id.expect("the invoice"));
    let found = invoicer(s.t.state())
        .find(s.t.db(), invoice)
        .await
        .unwrap()
        .expect("the invoice");
    assert_ne!(found.status, InvoiceStatus::Draft);
    let linked = Cheque::find_or_404(s.t.db(), cheque.id)
        .await
        .unwrap()
        .linked(s.t.db())
        .await
        .unwrap();
    assert_eq!(linked, vec![Subject::new("invoicing.invoice", invoice.0)]);
    let rows = ChequeWriteOff::of_cheque(s.t.db(), cheque.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].invoice_id, invoice.0);
}
