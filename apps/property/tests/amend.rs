//! BL-PRP-08 (#258): an amendment version that is activated ends the old
//! version as `amended`: the charges from the effective date are recreated
//! from the new plan, the invoices issued for those periods are credited and
//! reissued (a paid one is unallocated first; one that straddles the effective
//! date is split pro rata), the cheques are relinked or handed back, the
//! deposit is adjusted and `LeaseAmended` is emitted.

mod support;

use property::amend::{self, DEPOSIT_TOP_UP};
use property::leases::NewVersion;
use property::legal_case::LegalCase;
use property::{ChargeStatus, Lease, LeaseCharge, LeaseStatus, NewLease};
use renox::chrono::{Datelike, Duration, Months, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::cheques::{ChequeId, ChequeStatus};
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::payments::RefundSource;

use support::{AGENT, Booted, MANAGER, TENANT, aed, boot, d};

struct Scene {
    t: Booted,
    building: property::Building,
    unit: property::Unit,
    agent: User,
    manager: User,
}

async fn scene() -> Scene {
    let t = boot().await;
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let manager = t.user_in("Mona", MANAGER, building.id).await;
    Scene {
        t,
        building,
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
    /// monthly charges, with a deposit of AED 5,000 held.
    async fn leased(&self) -> Lease {
        let new = NewLease::new(self.unit.id, TENANT, d(1, 1), d(12, 31), aed(120_000), 12)
            .deposit(aed(5_000));
        let lease = property::leases::create_draft(self.t.state(), None, new)
            .await
            .unwrap();
        self.t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
        self.t.submit(&self.agent, &lease).await.assert_ok();
        self.t.activate(&self.manager, &lease).await.assert_ok();
        self.t.reload(&lease).await
    }

    /// The daily run drafts the invoices due by `day`; they are issued and
    /// paid.
    async fn invoice_and_pay(&self, day: NaiveDate) {
        property::invoicing::run(self.t.state(), day).await.unwrap();
        for invoice in self.t.invoices.all() {
            let id = invoice.summary.id.0;
            self.t.invoices.issue(id);
            self.t.invoices.pay(
                id,
                &Subject::new("payments.payment", id),
                invoice.summary.gross_total,
            );
        }
    }

    async fn charges(&self, lease: &Lease) -> Vec<LeaseCharge> {
        let mut all = lease.charges(self.t.db()).await.unwrap();
        all.sort_by_key(|c| (c.due_date, c.id));
        all
    }

    /// A cheque for each rent charge due on or after `from`.
    async fn cheques_from(&self, lease: &Lease, from: NaiveDate) -> Vec<ChequeId> {
        let mut made = Vec::new();
        for charge in self
            .charges(lease)
            .await
            .iter()
            .filter(|c| c.due_date >= from)
        {
            made.push(self.t.cheques.receive(
                TENANT,
                &lease.agreement(),
                &format!("C-{}", charge.due_date),
                charge.due_date,
                charge.amount,
                &[charge.subject()],
            ));
        }
        made
    }

    /// Sets an arrears flag on `lease`.
    async fn flag(&self, lease: &Lease) {
        self.t
            .post(
                &self.manager,
                &format!("/property/leases/{}/legal-cases", lease.id),
                &[
                    ("case_type", "arrears"),
                    ("reference", "RDC-2027-1234"),
                    ("ticket", "new"),
                ],
            )
            .await
            .assert_ok();
    }

    /// A draft amendment of `v1` from `effective` to 2028-04-30, in 12
    /// instalments, submitted (the tenant's acceptance is attached).
    async fn amendment(
        &self,
        v1: &Lease,
        effective: NaiveDate,
        rent: i64,
        deposit: i64,
        first_due: Option<NaiveDate>,
    ) -> Lease {
        let mut new = NewVersion::amendment(effective, ymd(2028, 4, 30), rent, deposit, 12);
        new.first_due = first_due;
        let v2 = property::leases::create_version(self.t.state(), Some(self.agent.id), v1.id, new)
            .await
            .unwrap();
        self.t.accept(&v2);
        self.t.submit(&self.agent, &v2).await.assert_ok();
        self.t.reload(&v2).await
    }
}

/// AC1, AC2: v1 active from 2027-01-01 at AED 120,000 a year in 12 monthly
/// charges, invoices issued to April and cheques `received` for May to
/// December. An amendment from 2027-05-01 at AED 132,000 ends v1 as `amended`,
/// cancels its May to December charges, creates May to December charges of AED
/// 11,000, hands the 8 cheques of AED 10,000 back against one receipt, gives the
/// accountant a task and emits `LeaseAmended`.
#[renox::test]
async fn bl_prp_08_an_amendment_recreates_the_charges_and_returns_the_cheques() {
    let s = scene().await;
    let v1 = s.leased().await;
    s.invoice_and_pay(d(4, 30)).await;
    let cheques = s.cheques_from(&v1, d(5, 1)).await;
    assert_eq!(cheques.len(), 8);
    let v2 = s
        .amendment(&v1, d(5, 1), aed(132_000), aed(5_000), None)
        .await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    let old = s.t.reload(&v1).await;
    assert_eq!(old.status, LeaseStatus::Amended);
    assert_eq!(old.next_id, Some(v2.id));
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Active);
    for charge in s.charges(&old).await {
        let expected = if charge.due_date >= d(5, 1) {
            ChargeStatus::Cancelled
        } else {
            ChargeStatus::Invoiced
        };
        assert_eq!(charge.status, expected, "{}", charge.due_date);
    }
    let new_charges = s.charges(&v2).await;
    assert_eq!(new_charges.len(), 12);
    for charge in new_charges.iter().filter(|c| c.due_date <= d(12, 1)) {
        assert_eq!(charge.amount, aed(11_000), "{}", charge.due_date);
        assert_eq!(charge.status, ChargeStatus::Scheduled);
    }
    assert!(
        s.t.invoices.credits().is_empty(),
        "nothing is invoiced after May"
    );

    // The 8 cheques go back against one receipt; the accountant collects others.
    for id in &cheques {
        assert_eq!(s.t.cheques.get(*id).status, ChequeStatus::Returned);
    }
    let returns = s.t.cheques.returns();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].cheques.len(), 8);
    let tasks = s.t.tasks.with_key(&amend::cheques_key(&v2));
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].title.contains("replacement cheques"),
        "{}",
        tasks[0].title
    );
    assert!(
        tasks[0].description.contains("HOR-TEST"),
        "{}",
        tasks[0].description
    );

    let events = s.t.amended.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].lease, v2.id);
    assert_eq!(events[0].previous, v1.id);
    assert_eq!(events[0].effective_date, d(5, 1));
    assert_eq!(events[0].annual_rent, aed(132_000));
    assert_eq!(events[0].returned_cheques, 8);

    // PRP.RUL.02: the Ejari task of the amendment, due 14 days from now.
    let ejari = s.t.tasks.with_key(&format!("property.ejari:{}", v2.id));
    assert_eq!(ejari.len(), 1);
    assert!(
        ejari[0].title.contains("Update the Ejari"),
        "{}",
        ejari[0].title
    );

    // The effects run once.
    let again = amend::settle(s.t.state(), v2.id, None).await.unwrap();
    assert!(!again.ran);
    assert_eq!(s.t.cheques.returns().len(), 1);
    assert_eq!(s.t.amended.all().len(), 1);
}

/// A `received` cheque whose new charge has the same amount is relinked to it,
/// not returned; a deposited one is never touched.
#[renox::test]
async fn bl_prp_08_cheques_of_the_same_amount_are_relinked_and_deposited_ones_stay() {
    let s = scene().await;
    let v1 = s.leased().await;
    let cheques = s.cheques_from(&v1, d(5, 1)).await;
    // The May cheque was deposited already.
    s.t.cheques.set_status(cheques[0], ChequeStatus::Deposited);
    // The rent stays, the deposit goes up: the charges have the same amounts.
    let v2 = s
        .amendment(&v1, d(5, 1), aed(120_000), aed(6_000), None)
        .await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    assert_eq!(s.t.cheques.get(cheques[0]).status, ChequeStatus::Deposited);
    let new_charges = s.charges(&v2).await;
    let rent: Vec<&LeaseCharge> = new_charges
        .iter()
        .filter(|c| c.charge_type == "rent")
        .collect();
    for (index, id) in cheques.iter().enumerate().skip(1) {
        let cheque = s.t.cheques.get(*id);
        assert_eq!(cheque.status, ChequeStatus::Received, "{index}");
        let target = rent
            .iter()
            .find(|c| c.due_date == cheque.date)
            .expect("a charge of the date");
        assert_eq!(cheque.linked, vec![target.subject()], "{}", cheque.date);
    }
    // The deposited cheque still pays the cancelled charge of v1.
    let old_charge = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.due_date == d(5, 1))
        .unwrap();
    assert_eq!(
        s.t.cheques.get(cheques[0]).linked,
        vec![old_charge.subject()]
    );
    assert!(s.t.cheques.returns().is_empty());
    assert!(s.t.tasks.with_key(&amend::cheques_key(&v2)).is_empty());
}

/// AC3: with an effective date of 2027-04-15 the April invoice (AED 10,000,
/// paid) is unallocated, credited and reissued as AED 4,666.67 (14 of 30 days
/// at the old rate) plus AED 5,866.67 (16 days at the new rate); the payment
/// goes back on the new invoice.
#[renox::test]
async fn bl_prp_08_an_invoice_that_straddles_the_effective_date_is_split_pro_rata() {
    let s = scene().await;
    let v1 = s.leased().await;
    s.invoice_and_pay(d(4, 1)).await;
    let april = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.due_date == d(4, 1))
        .unwrap();
    let old_invoice = april.invoice_id.expect("the April invoice");
    // The new plan starts with the May charge.
    let v2 = s
        .amendment(&v1, d(4, 15), aed(132_000), aed(5_000), Some(d(5, 1)))
        .await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    // The old invoice is credited; January to March are left alone.
    let credited = s.t.invoices.get(old_invoice).unwrap();
    assert_eq!(credited.summary.status, InvoiceStatus::Credited);
    let others =
        s.t.invoices
            .all()
            .into_iter()
            .filter(|i| i.summary.status == InvoiceStatus::Credited)
            .count();
    assert_eq!(others, 1);

    // The reissued invoice has two lines.
    let reissued: Vec<_> =
        s.t.invoices
            .all()
            .into_iter()
            .filter(|i| i.draft.lines.len() == 2)
            .collect();
    assert_eq!(reissued.len(), 1);
    let nets: Vec<i64> = reissued[0].draft.lines.iter().map(|l| l.net()).collect();
    assert_eq!(nets, vec![466_667, 586_667]);
    assert_eq!(reissued[0].summary.gross_total, 1_053_334);
    // The payment taken off the old invoice is allocated to the new one.
    assert_eq!(
        reissued[0].allocations,
        vec![(Subject::new("payments.payment", old_invoice), aed(10_000))]
    );
    let stands = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.id == april.id)
        .unwrap();
    assert_eq!(stands.invoice_id, Some(reissued[0].summary.id.0));
    assert_eq!(stands.status, ChargeStatus::Invoiced);
}

/// An invoice for a period after the effective date, issued and paid, is
/// unallocated, credited in full and reissued from the amendment's charge for
/// the period; the payment is allocated to the new invoice, which is AED 1,000
/// more.
#[renox::test]
async fn bl_prp_08_a_paid_invoice_after_the_effective_date_is_credited_and_reissued() {
    let s = scene().await;
    let first_of_month = today().with_day(1).unwrap();
    let start = first_of_month.checked_sub_months(Months::new(2)).unwrap();
    let end = start.checked_add_months(Months::new(12)).unwrap() - Duration::days(1);
    let new = NewLease::new(s.unit.id, TENANT, start, end, aed(120_000), 12);
    let v1 = property::leases::create_draft(s.t.state(), None, new)
        .await
        .unwrap();
    s.t.submit(&s.agent, &v1).await.assert_ok();
    s.t.activate(&s.manager, &v1).await.assert_ok();
    let v1 = s.t.reload(&v1).await;
    s.invoice_and_pay(today()).await;
    let this_month = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.due_date == first_of_month)
        .unwrap();
    let old_invoice = this_month.invoice_id.expect("this month's invoice");

    let mut change = NewVersion::amendment(first_of_month, end, aed(132_000), 0, 12);
    change.first_due = None;
    let v2 = property::leases::create_version(s.t.state(), Some(s.agent.id), v1.id, change)
        .await
        .unwrap();
    s.t.accept(&v2);
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.t.activate(&s.manager, &v2).await.assert_ok();

    // The invoices of the two earlier months stand.
    let credited: Vec<i64> =
        s.t.invoices
            .all()
            .into_iter()
            .filter(|i| i.summary.status == InvoiceStatus::Credited)
            .map(|i| i.summary.id.0)
            .collect();
    assert_eq!(credited, vec![old_invoice]);
    // The amendment's charge of this month is invoiced at the new rate.
    let new_charge = s
        .charges(&v2)
        .await
        .into_iter()
        .find(|c| c.due_date == first_of_month)
        .unwrap();
    assert_eq!(new_charge.status, ChargeStatus::Invoiced);
    let new_invoice = s.t.invoices.get(new_charge.invoice_id.unwrap()).unwrap();
    assert_eq!(new_invoice.summary.gross_total, aed(11_000));
    assert_eq!(
        new_invoice.allocations,
        vec![(Subject::new("payments.payment", old_invoice), aed(10_000))]
    );
    assert_eq!(new_invoice.summary.balance, aed(1_000));
    // Nothing is left to finish by hand.
    assert!(s.t.tasks.with_key(&amend::problems_key(&v2)).is_empty());
}

/// A draft invoice of a period after the effective date is discarded and its
/// charge cancelled; the amendment's charge is invoiced on its due date.
#[renox::test]
async fn bl_prp_08_a_draft_invoice_after_the_effective_date_is_discarded() {
    let s = scene().await;
    let v1 = s.leased().await;
    s.invoice_and_pay(d(4, 30)).await;
    // May is drafted, not issued.
    property::invoicing::run(s.t.state(), d(5, 1))
        .await
        .unwrap();
    let may = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.due_date == d(5, 1))
        .unwrap();
    let draft_id = may.invoice_id.expect("the May draft");
    let v2 = s
        .amendment(&v1, d(5, 1), aed(132_000), aed(5_000), None)
        .await;

    s.t.activate(&s.manager, &v2).await.assert_ok();

    assert!(
        s.t.invoices.get(draft_id).is_none(),
        "the draft is discarded"
    );
    let old = s
        .charges(&v1)
        .await
        .into_iter()
        .find(|c| c.id == may.id)
        .unwrap();
    assert_eq!(old.status, ChargeStatus::Cancelled);
    let new_may = s
        .charges(&v2)
        .await
        .into_iter()
        .find(|c| c.due_date == d(5, 1))
        .unwrap();
    assert_eq!(new_may.status, ChargeStatus::Scheduled);
}

/// AC4: a deposit raised from AED 5,000 to AED 6,000 runs `adjust` and creates
/// a top-up charge of AED 1,000 dated the effective date; one lowered to AED
/// 4,000 asks for a refund of AED 1,000 from the held deposit; in both cases the
/// deposit stays held.
#[renox::test]
async fn bl_prp_08_a_changed_deposit_runs_adjust() {
    let up = scene().await;
    let v1 = up.leased().await;
    let v2 = up
        .amendment(&v1, d(5, 1), aed(120_000), aed(6_000), None)
        .await;
    up.t.activate(&up.manager, &v2).await.assert_ok();

    let asked = up.t.deposits.adjusted();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].new_amount, aed(6_000));
    assert_eq!(asked[0].source, v1.agreement());
    assert_eq!(asked[0].source_version, 2);
    assert_eq!(asked[0].effective_on, d(5, 1));
    let top_up: Vec<LeaseCharge> = up
        .charges(&v2)
        .await
        .into_iter()
        .filter(|c| c.charge_type == DEPOSIT_TOP_UP)
        .collect();
    assert_eq!(top_up.len(), 1);
    assert_eq!(top_up[0].amount, aed(1_000));
    assert_eq!(top_up[0].due_date, d(5, 1));
    assert_eq!(top_up[0].status, ChargeStatus::Scheduled);
    assert!(up.t.tasks.with_key("property.deposit_adjust").is_empty());
    assert!(up.t.refunds.asked().is_empty());

    let down = scene().await;
    let v1 = down.leased().await;
    let v2 = down
        .amendment(&v1, d(5, 1), aed(120_000), aed(4_000), None)
        .await;
    down.t.activate(&down.manager, &v2).await.assert_ok();
    let refunds = down.t.refunds.asked();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, aed(1_000));
    assert!(matches!(refunds[0].source, RefundSource::Deposit(_)));
    assert!(
        down.charges(&v2)
            .await
            .iter()
            .all(|c| c.charge_type != DEPOSIT_TOP_UP)
    );
}

/// AC5: with a legal case flag open on v1, `submit` of v2 is refused with
/// "legal case open"; a version submitted before the flag cannot be activated
/// after it.
#[renox::test]
async fn bl_prp_08_a_legal_case_refuses_submit_and_activate() {
    let s = scene().await;
    let v1 = s.leased().await;
    let mut change = NewVersion::amendment(d(5, 1), ymd(2028, 4, 30), aed(132_000), aed(5_000), 12);
    change.first_due = None;
    let v2 = property::leases::create_version(s.t.state(), Some(s.agent.id), v1.id, change)
        .await
        .unwrap();
    s.t.accept(&v2);
    s.flag(&v1).await;

    let refused = s.t.submit(&s.agent, &v2).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Legal case open"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&v2).await, LeaseStatus::Draft);

    // Cleared, it is submitted; set again, it cannot be activated.
    let flag = LegalCase::of_root(s.t.db(), v1.root_id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    s.t.post(
        &s.manager,
        &format!("/property/leases/{}/legal-cases/clear", v1.id),
        &[("case", flag.id.to_string().as_str()), ("outcome", "paid")],
    )
    .await
    .assert_ok();
    s.t.submit(&s.agent, &v2).await.assert_ok();
    s.flag(&v1).await;
    let refused = s.t.activate(&s.manager, &v2).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Legal case open"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&v2).await, LeaseStatus::PendingSignature);
    assert_eq!(s.t.status(&v1).await, LeaseStatus::Active);
}

/// AC6: an amendment draft with an effective date in the previous month is
/// refused at `submit`.
#[renox::test]
async fn bl_prp_08_an_effective_date_in_the_previous_month_is_refused() {
    let s = scene().await;
    let v1 = s.t.active_lease(&s.unit, &s.agent, &s.manager).await;
    let last_month = today().with_day(1).unwrap() - Duration::days(1);
    let draft = property::leases::create_version(
        s.t.state(),
        None,
        v1.id,
        NewVersion::amendment(last_month, v1.end_date, aed(105_000), 0, 2),
    )
    .await
    .unwrap();
    s.t.accept(&draft);
    let refused = s.t.submit(&s.agent, &draft).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("first day of this month"),
        "{}",
        refused.text()
    );
    assert_eq!(s.t.status(&draft).await, LeaseStatus::Draft);
    let _ = &s.building;
}
