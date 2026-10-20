//! The scheduled lease charges in the cash-flow forecast of wl-invoicing, with
//! the real money modules (wl-invoicing computes the forecast and runs the
//! alert, wl-payments keeps the cheques): 01 Property adds each *scheduled*
//! charge not yet invoiced as a receipt through the kernel
//! `CashForecastSource`, a post-dated cheque linked to the charge replaces it,
//! and an invoiced charge counts through its invoice only. Checks of BL-INV-28
//! (#91) "Received PDC CHQ-7 … linked to charge CH-3 … appears in week 4 only"
//! and BL-INV-30 (#93) "A scheduled charge of AED 10,000 due in week 6, not
//! yet invoiced, is counted in the balance the rule checks".
//!
//! Today is Wednesday 2027-01-13; the weeks start on Monday, so week 1 is
//! 2027-01-11 to 2027-01-17, week 2 01-18 to 01-24, week 4 02-01 to 02-07 and
//! week 6 02-15 to 02-21. Every unit is residential: rent is exempt from VAT.

mod support;

use property::cash_forecast::{CHARGES_CATEGORY, CHARGES_SOURCE};
use property::cheques::{ChequeEntry, register_cheques};
use property::{ChargeStatus, Lease, LeaseCharge, NewLease};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast, Line, LineKind};
use wl_invoicing::{InvoicingConfig, SupplierBill, SupplierBillStatus, WlInvoicing};
use wl_kernel::LegalEntity;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};

use support::{AGENT, Booted, COMPANY, MANAGER, TENANT, aed, boot_money, d};

/// The threshold of PGP in the alert test: AED 20,000.
const THRESHOLD: i64 = 2_000_000;

struct Scene {
    t: Booted,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene(invoicing: WlInvoicing) -> Scene {
    let t = boot_money(invoicing.auto_issue_source("property.lease")).await;
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

impl Scene {
    /// A draft lease of a year from `start` at `annual` fils in 12 monthly
    /// charges, with no deposit.
    async fn draft(&self, start: NaiveDate, annual: i64) -> Lease {
        let end = start
            .checked_add_months(renox::chrono::Months::new(12))
            .unwrap()
            .pred_opt()
            .unwrap();
        let new = NewLease::new(self.unit.id, TENANT, start, end, annual, 12);
        property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap()
    }

    /// Submits and activates `lease`: its charges are *scheduled*.
    async fn activate(&self, lease: &Lease) -> Lease {
        self.t.submit(&self.agent, lease).await.assert_ok();
        self.t.activate(&self.manager, lease).await.assert_ok();
        let active = self.t.reload(lease).await;
        assert!(
            LeaseCharge::of_lease(self.t.db(), active.id)
                .await
                .unwrap()
                .iter()
                .all(|c| c.status == ChargeStatus::Scheduled)
        );
        active
    }

    async fn charges(&self, lease: &Lease) -> Vec<LeaseCharge> {
        LeaseCharge::of_lease(self.t.db(), lease.id).await.unwrap()
    }

    async fn entity(&self) -> LegalEntity {
        LegalEntity::default_entity(self.t.db()).await.unwrap()
    }

    /// The forecast of the default entity as wl-invoicing computes it on
    /// `on` (inside the travelled clock).
    async fn forecast(&self, on: NaiveDate) -> Forecast {
        let config = self
            .t
            .state()
            .provided::<InvoicingConfig>()
            .expect("the invoicing config");
        let entities = vec![self.entity().await];
        let inputs = wl_invoicing::cash_forecast_load::load(self.t.state(), &config, &entities, on)
            .await
            .unwrap();
        cash_forecast::assemble(&inputs)
    }
}

/// The lines of the scheduled charges.
fn charge_lines(forecast: &Forecast) -> Vec<&Line> {
    forecast
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Source && l.category == CHARGES_CATEGORY)
        .collect()
}

/// #91: the lease of 2027-01-20 has 12 charges of AED 8,000; the first (CH-3
/// of the issue) is due 2027-01-20, in week 2. Not yet invoiced, it is a
/// receipt of the tenant in week 2 naming the charge. Once post-dated cheque
/// CHQ-7 of AED 8,000 dated 2027-02-01 is registered in wl-payments linked to
/// it, the cheque counts in week 4 and the charge does not appear.
#[renox::test]
async fn a_cheque_linked_to_a_scheduled_lease_charge_replaces_it_in_the_forecast() {
    let s = scene(WlInvoicing::new()).await;
    let lease = s.draft(d(1, 20), aed(96_000)).await;
    // A lease being prepared has no scheduled charge yet: nothing to forecast.
    let draft = s.t.at(d(1, 13), s.forecast(d(1, 13))).await;
    assert!(charge_lines(&draft).is_empty());

    let lease = s.activate(&lease).await;
    let charges = s.charges(&lease).await;
    let ch3 = &charges[0];
    assert_eq!((ch3.due_date, ch3.amount), (d(1, 20), aed(8_000)));

    let before = s.t.at(d(1, 13), s.forecast(d(1, 13))).await;
    let lines = charge_lines(&before);
    let first = lines
        .iter()
        .find(|l| l.documents == vec![ch3.subject()])
        .expect("CH-3 is forecast");
    assert_eq!(first.week, 1, "week 2");
    assert_eq!(first.amount, aed(8_000));
    assert_eq!(first.party, Some(TENANT));
    assert_eq!(first.source.as_deref(), Some(CHARGES_SOURCE));
    assert_eq!(before.receipts(1), aed(8_000));
    // The other charges fall within the 13 weeks once a month.
    assert_eq!(lines.len(), 3);
    assert_eq!(before.receipts(3), 0);

    let registered = register_cheques(
        s.t.state(),
        None,
        &lease,
        "ENBD",
        vec![ChequeEntry::new("CHQ-7", d(2, 1), aed(8_000), Some(ch3.id))],
    )
    .await
    .unwrap();
    assert_eq!(registered.len(), 1);

    let after = s.t.at(d(1, 13), s.forecast(d(1, 13))).await;
    assert_eq!(after.receipts(3), aed(8_000), "week 4: the cheque");
    assert_eq!(after.receipts(1), 0, "week 2: CH-3 is gone");
    let cheque = after
        .lines
        .iter()
        .find(|l| l.kind == LineKind::ChequeIn)
        .expect("the cheque line");
    assert_eq!(cheque.week, 3);
    assert!(cheque.documents.contains(&ch3.subject()));
    assert!(
        charge_lines(&after)
            .iter()
            .all(|l| !l.documents.contains(&ch3.subject()))
    );
    assert_eq!(charge_lines(&after).len(), 2);
}

/// A charge the daily run invoiced counts once: on 2027-01-20 CH-3 becomes a
/// draft invoice of AED 8,000, which wl-invoicing does not count, so the
/// charge stays a line of week 1 (01-18 to 01-24); once the invoice is issued
/// it counts through the invoice and no charge line names it.
#[renox::test]
async fn an_invoiced_lease_charge_counts_through_its_invoice_only() {
    let s = scene(WlInvoicing::new()).await;
    let lease = s.draft(d(1, 20), aed(96_000)).await;
    let lease = s.activate(&lease).await;
    let ch3 = s.charges(&lease).await[0].clone();

    let drafted =
        s.t.at(d(1, 20), async {
            let report = property::invoicing::run(s.t.state(), d(1, 20))
                .await
                .unwrap();
            assert_eq!(report.invoiced, 1);
            s.forecast(d(1, 20)).await
        })
        .await;
    let ch3 = LeaseCharge::find_or_404(s.t.db(), ch3.id).await.unwrap();
    assert_eq!(ch3.status, ChargeStatus::Invoiced);
    let invoice = InvoiceId(ch3.invoice_id.expect("the invoice"));
    let found = invoicer(s.t.state())
        .find(s.t.db(), invoice)
        .await
        .unwrap()
        .expect("the invoice");
    assert_eq!(found.status, InvoiceStatus::Draft);
    assert_eq!(drafted.receipts(0), aed(8_000), "once, through the charge");
    assert!(drafted.lines.iter().all(|l| l.kind != LineKind::Invoice));
    assert!(
        charge_lines(&drafted)
            .iter()
            .any(|l| l.documents.contains(&ch3.subject()))
    );

    let issued =
        s.t.at(d(1, 20), async {
            invoicer(s.t.state())
                .issue(s.t.state(), invoice, None)
                .await
                .unwrap();
            s.forecast(d(1, 20)).await
        })
        .await;
    assert_eq!(issued.receipts(0), aed(8_000), "once, through the invoice");
    let line = issued
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Invoice)
        .expect("the invoice line");
    assert_eq!(line.amount, aed(8_000));
    assert_eq!(line.documents[0].subject_id, invoice.0);
    assert!(
        charge_lines(&issued)
            .iter()
            .all(|l| !l.documents.contains(&ch3.subject()))
    );
}

/// #91, "must not count the same money twice (… invoice and its PDC)": CH-3
/// has post-dated cheque CHQ-7 of AED 8,000 dated 2027-02-01 linked to it
/// (`property.lease_charge:<id>`). Once the daily run of 2027-01-20 has
/// invoiced CH-3 and the invoice is issued, the cheque still names the
/// charge, and wl-invoicing reads it as covering the invoice that bills the
/// charge: the AED 8,000 counts once, through the cheque by its date (week 3,
/// 02-01 to 02-07, seen from 2027-01-20), and the invoice adds nothing.
#[renox::test]
async fn an_issued_invoice_of_a_charge_with_a_linked_cheque_counts_once() {
    let s = scene(WlInvoicing::new()).await;
    let lease = s.draft(d(1, 20), aed(96_000)).await;
    let lease = s.activate(&lease).await;
    let ch3 = s.charges(&lease).await[0].clone();
    register_cheques(
        s.t.state(),
        None,
        &lease,
        "ENBD",
        vec![ChequeEntry::new("CHQ-7", d(2, 1), aed(8_000), Some(ch3.id))],
    )
    .await
    .unwrap();

    let forecast =
        s.t.at(d(1, 20), async {
            let report = property::invoicing::run(s.t.state(), d(1, 20))
                .await
                .unwrap();
            assert_eq!(report.invoiced, 1);
            let charge = LeaseCharge::find_or_404(s.t.db(), ch3.id).await.unwrap();
            let invoice = InvoiceId(charge.invoice_id.expect("the invoice"));
            let found = invoicer(s.t.state())
                .find(s.t.db(), invoice)
                .await
                .unwrap()
                .expect("the invoice");
            if found.status == InvoiceStatus::Draft {
                invoicer(s.t.state())
                    .issue(s.t.state(), invoice, None)
                    .await
                    .unwrap();
            }
            let found = invoicer(s.t.state())
                .find(s.t.db(), invoice)
                .await
                .unwrap()
                .expect("the invoice");
            assert_eq!(found.status, InvoiceStatus::Issued);
            assert_eq!(found.balance, aed(8_000));
            s.forecast(d(1, 20)).await
        })
        .await;
    let charge = LeaseCharge::find_or_404(s.t.db(), ch3.id).await.unwrap();
    let invoice = InvoiceId(charge.invoice_id.expect("the invoice")).subject();

    // Every line that stands for CH-3, its invoice or CHQ-7 adds up to the
    // AED 8,000 once.
    let ours: Vec<&Line> = forecast
        .lines
        .iter()
        .chain(forecast.after.iter())
        .filter(|l| {
            l.kind == LineKind::ChequeIn
                || l.documents.contains(&invoice)
                || l.documents.contains(&ch3.subject())
        })
        .collect();
    assert_eq!(ours.len(), 1, "one line: {ours:?}");
    assert_eq!(ours.iter().map(|l| l.amount).sum::<i64>(), aed(8_000));
    let cheque = ours[0];
    assert_eq!(cheque.kind, LineKind::ChequeIn);
    assert_eq!(cheque.week, 2, "week 3, by the cheque date");
    assert!(cheque.documents.contains(&invoice));
    assert!(forecast.lines.iter().all(|l| l.kind != LineKind::Invoice));
    assert_eq!(
        forecast.receipts(0),
        0,
        "week 1: neither the invoice nor CH-3"
    );
    assert_eq!(forecast.receipts(2), aed(8_000));
    let total: i64 = (0..forecast.weeks.len())
        .map(|w| forecast.receipts(w))
        .sum();
    // The two later charges of the lease within the 13 weeks, and CHQ-7.
    assert_eq!(total, aed(24_000));
}

/// #93: PGP has AED 50,000 in the bank on 2027-01-12, a threshold of AED
/// 20,000 and a supplier bill of AED 35,000 due 2027-02-16: week 6 is
/// projected at AED 15,000 and the 06:00 run warns for week 6 while the lease
/// of 2027-02-15 is only drafted. Once it is active its first charge of AED
/// 10,000, due 2027-02-15 and not yet invoiced, is counted: week 6 is at AED
/// 25,000, no week is below and the next run tells nobody.
#[renox::test]
async fn a_scheduled_lease_charge_not_yet_invoiced_counts_in_the_balance_the_alert_checks() {
    let s = scene(WlInvoicing::new().cash_threshold("PGP", THRESHOLD)).await;
    s.t.user_with("Boss", wl_kernel::COMPANY_ADMIN_ROLE).await;
    let entity = s.entity().await;
    s.t.at(d(1, 13), async {
        cash_balance::enter(
            s.t.db(),
            None,
            &EnterBalance {
                legal_entity_id: entity.id,
                bank_account: "BA1".to_owned(),
                amount: aed(50_000),
                as_of: d(1, 12),
                note: None,
            },
        )
        .await
        .unwrap();
    })
    .await;
    let mut bill = SupplierBill {
        party_id: COMPANY.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: "SB-W6".to_owned(),
        number_key: "sb-w6".to_owned(),
        bill_date: d(1, 2),
        due_date: Some(d(2, 16)),
        currency: "AED".to_owned(),
        gross_total: aed(35_000),
        balance: aed(35_000),
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    let mut tx = s.t.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, SupplierBillStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let lease = s.draft(d(2, 15), aed(120_000)).await;
    let sent =
        s.t.at(d(1, 13), cash_alert::run(s.t.state(), d(1, 13)))
            .await
            .unwrap();
    assert_eq!(sent, 1, "without the charge week 6 is below");
    let notices = s.t.notices.of_type(BELOW_THRESHOLD);
    assert_eq!(notices[0].data["week"], 6);

    let lease = s.activate(&lease).await;
    let charge = s.charges(&lease).await[0].clone();
    assert_eq!((charge.due_date, charge.amount), (d(2, 15), aed(10_000)));
    let forecast = s.t.at(d(1, 14), s.forecast(d(1, 14))).await;
    assert_eq!(forecast.weeks[5].start, d(2, 15));
    assert_eq!(forecast.balance_for_alert(5, false), aed(25_000));
    assert_eq!(
        cash_alert::first_week_below(&forecast, THRESHOLD, false),
        None
    );
    let sent =
        s.t.at(d(1, 14), cash_alert::run(s.t.state(), d(1, 14)))
            .await
            .unwrap();
    assert_eq!(sent, 0);
    assert_eq!(s.t.notices.of_type(BELOW_THRESHOLD).len(), 1);
    // The charge is still only scheduled: the forecast did not change it.
    let charge = LeaseCharge::find_or_404(s.t.db(), charge.id).await.unwrap();
    assert_eq!(charge.status, ChargeStatus::Scheduled);
}
