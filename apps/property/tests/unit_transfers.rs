//! BL-PRP-23 (#273): `transfer_unit`. A unit leaves a management agreement
//! because it is sold (the company still manages it for the buyer) or is no
//! longer managed. A property manager asks, another approves; nothing changes
//! before.

mod support;

use property::agreement::{
    self, AgreementStatus, AgreementUnit, ManagementAgreement, NewAgreement,
};
use property::agreement_lifecycle::HandoverItem;
use property::notices::PAYEE_CHANGED;
use property::statement::{self, LineKind, StatementLine};
use property::unit_transfer::{TransferStatus, UnitTransfer, empty_task_key};
use property::{ChargeStatus, Lease, LeaseCharge, Unit};
use renox::chrono::{Datelike, Duration, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::cheques::{ChequeId, ChequeStatus};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::parties::PartyKind;
use wl_kernel::{PartyId, Subject};

use support::{AGENT, Booted, MANAGER, Real, TENANT, aed, boot, boot_with, day};

const OWNER: PartyId = PartyId(31);
const BUYER: PartyId = PartyId(41);

struct Scene {
    /// The transfer date.
    at: NaiveDate,
    t: Booted,
    old: ManagementAgreement,
    unit: Unit,
    lease: Lease,
    cheque: ChequeId,
    first: User,
    second: User,
}

async fn scene() -> Scene {
    scene_on(boot().await, false).await
}

/// The scene on `t`; the transfer is dated 60 days ago, or with
/// `first_of_month` the first day of the month of the lease's last instalment.
async fn scene_on(t: Booted, first_of_month: bool) -> Scene {
    t.parties
        .add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    t.parties.add(BUYER.0, "Omar Khalid", PartyKind::Person);
    let building = t.building().await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site = t.user_in("Sami", MANAGER, building.id).await;
    let unit = t.unit(&building, "803").await;
    let lease = t.active_lease(&unit, &agent, &site).await;
    t.deposits.hold(&lease.agreement(), TENANT, aed(5_000));
    let charges = LeaseCharge::of_lease(t.db(), lease.id).await.unwrap();
    let last = charges.last().expect("a charge").clone();
    let at = if first_of_month {
        month_of(last.due_date).0
    } else {
        day(-60)
    };
    assert!(last.due_date >= at);
    let cheque = t.cheques.receive(
        TENANT,
        &lease.agreement(),
        "803-4",
        last.due_date,
        aed(25_000),
        &[last.subject()],
    );
    let mut old = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    old.insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    agreement::add_unit(t.db(), &old, unit.id, day(-400))
        .await
        .unwrap();
    let first = t.user_with("Mona", MANAGER).await;
    let second = t.user_with("Mira", MANAGER).await;
    Scene {
        at,
        t,
        old,
        unit,
        lease,
        cheque,
        first,
        second,
    }
}

impl Scene {
    fn date(&self) -> String {
        self.at.to_string()
    }

    async fn request(&self, new_owner: &str) -> renox::testing::TestResponse {
        let date = self.date();
        self.t
            .post(
                &self.first,
                &format!("/property/agreements/{}/transfers", self.old.id),
                &[
                    ("unit", self.unit.id.to_string().as_str()),
                    ("date", date.as_str()),
                    ("new_owner", new_owner),
                    ("note", ""),
                ],
            )
            .await
    }

    async fn transfer(&self) -> UnitTransfer {
        UnitTransfer::of_agreement(self.t.db(), self.old.id)
            .await
            .unwrap()
            .remove(0)
    }

    async fn approve(&self, user: &User, receipt: &str) -> renox::testing::TestResponse {
        let id = self.transfer().await.id;
        self.t
            .post(
                user,
                &format!("/property/transfers/{id}/approve"),
                &[("receipt", receipt)],
            )
            .await
    }

    async fn buyer_agreement(&self, activate: bool) -> ManagementAgreement {
        let mut draft =
            agreement::create_draft(self.t.state(), None, NewAgreement::new(BUYER, self.at, 400))
                .await
                .unwrap();
        ManagementAgreement::query()
            .where_(ManagementAgreement::ID.eq(draft.id))
            .update(self.t.db(), &[("legal_entity_id", &Some(2_i64))])
            .await
            .unwrap();
        agreement::add_unit(self.t.db(), &draft, self.unit.id, self.at)
            .await
            .unwrap();
        if activate {
            self.t
                .post(
                    &self.first,
                    &format!("/property/agreements/{}/activate", draft.id),
                    &[],
                )
                .await
                .assert_ok();
        }
        draft = ManagementAgreement::find_or_404(self.t.db(), draft.id)
            .await
            .unwrap();
        draft
    }
}

/// AC1, AC2, AC5: a sale still managed is refused until the buyer's agreement
/// is active; once approved by another manager the charges from the date are
/// recreated with the buyer's entity, April's invoice stays, the unit moves.
#[renox::test]
async fn bl_prp_23_a_managed_sale_moves_the_unit_and_recreates_the_charges() {
    let s = scene().await;
    let buyer = s.buyer_agreement(false).await;
    assert_eq!(buyer.status, AgreementStatus::Draft);
    let refused = s.request(&BUYER.0.to_string()).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .replace("&#39;", "'")
            .to_lowercase()
            .contains("new owner's agreement not active"),
        "{}",
        refused.text()
    );
    assert!(
        UnitTransfer::of_agreement(s.t.db(), s.old.id)
            .await
            .unwrap()
            .is_empty()
    );

    s.t.post(
        &s.first,
        &format!("/property/agreements/{}/activate", buyer.id),
        &[],
    )
    .await
    .assert_ok();
    s.request(&BUYER.0.to_string()).await.assert_ok();
    let asked = s.transfer().await;
    assert_eq!(asked.status, TransferStatus::Requested);

    // Without approval nothing changes.
    let rows = s.old.units(s.t.db()).await.unwrap();
    assert!(rows[0].date_out.is_none());
    s.approve(&s.first, "NEW-OWNER-RCPT")
        .await
        .assert_status(422);
    s.approve(&s.second, "").await.assert_status(422);
    assert_eq!(s.transfer().await.status, TransferStatus::Requested);

    s.approve(&s.second, "NEW-OWNER-RCPT").await.assert_ok();
    assert_eq!(s.transfer().await.status, TransferStatus::Approved);
    assert_eq!(
        s.transfer().await.deposit_receipt.as_deref(),
        Some("NEW-OWNER-RCPT")
    );

    let date = day(-60);
    let charges = LeaseCharge::of_lease(s.t.db(), s.lease.id).await.unwrap();
    let cancelled: Vec<_> = charges
        .iter()
        .filter(|c| c.status == ChargeStatus::Cancelled)
        .collect();
    let fresh: Vec<_> = charges
        .iter()
        .filter(|c| c.legal_entity_id == Some(2))
        .collect();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].status, ChargeStatus::Scheduled);
    assert_eq!(fresh[0].due_date, cancelled[0].due_date);
    assert!(
        charges
            .iter()
            .filter(|c| c.due_date < date)
            .all(|c| c.legal_entity_id.is_none())
    );

    // The cheque follows the recreated charge; the deposit stays held.
    assert!(
        s.t.cheques
            .get(s.cheque)
            .linked
            .contains(&fresh[0].subject())
    );
    assert!(s.t.deposits.is_held(&s.lease.agreement()));

    // The old owner's statements stop the day before; the buyer's start on it.
    let old_rows = s.old.units(s.t.db()).await.unwrap();
    assert_eq!(old_rows[0].date_out, Some(date - Duration::days(1)));
    let new_rows: Vec<AgreementUnit> = buyer.units(s.t.db()).await.unwrap();
    assert!(!new_rows[0].awaiting_transfer);
    let before = ManagementAgreement::for_unit(s.t.db(), s.unit.id, date - Duration::days(1))
        .await
        .unwrap();
    let after = ManagementAgreement::for_unit(s.t.db(), s.unit.id, date)
        .await
        .unwrap();
    assert_eq!(before.map(|a| a.id), Some(s.old.id));
    assert_eq!(after.map(|a| a.id), Some(buyer.id));

    assert_eq!(s.t.notices.of_type(PAYEE_CHANGED).len(), 1);
    // The last unit left: the property managers are told to give notice.
    assert_eq!(s.t.tasks.with_key(&empty_task_key(&s.old)).len(), 1);
    assert_eq!(
        ManagementAgreement::find_or_404(s.t.db(), s.old.id)
            .await
            .unwrap()
            .status,
        AgreementStatus::Active
    );
}

/// AC3: a "no longer managed" transfer returns the undeposited cheques and
/// transfers the deposit against a receipt.
#[renox::test]
async fn bl_prp_23_no_longer_managed_hands_the_unit_over() {
    let s = scene().await;
    s.request("").await.assert_ok();
    let id = s.transfer().await.id;
    let items = HandoverItem::of_transfer(s.t.db(), id).await.unwrap();
    assert_eq!(items.len(), 2);

    // A line without a receipt refuses the approval.
    s.approve(&s.second, "").await.assert_status(422);
    for item in &items {
        s.t.post(
            &s.first,
            &format!("/property/agreements/{}/transfer-receipts", s.old.id),
            &[
                ("item", item.id.to_string().as_str()),
                ("reference", "RCPT-X"),
            ],
        )
        .await
        .assert_ok();
    }
    s.approve(&s.second, "").await.assert_ok();

    assert_eq!(s.t.cheques.get(s.cheque).status, ChequeStatus::Returned);
    assert_eq!(s.t.cheques.returns()[0].reason, "handed over");
    assert_eq!(s.t.deposits.transferred().len(), 1);
    assert_eq!(s.t.deposits.transferred()[0].0.receipt, "RCPT-X");
    let lease = Lease::find_or_404(s.t.db(), s.lease.id).await.unwrap();
    assert_eq!(lease.handed_over_on, Some(day(-60)));
    assert_eq!(s.t.notices.of_type(PAYEE_CHANGED).len(), 1);
}

/// A rejected request changes nothing; the requester cannot approve it.
#[renox::test]
async fn bl_prp_23_a_rejected_request_changes_nothing() {
    let s = scene().await;
    s.request("").await.assert_ok();
    let id = s.transfer().await.id;
    s.t.post(&s.second, &format!("/property/transfers/{id}/reject"), &[])
        .await
        .assert_ok();
    assert_eq!(s.transfer().await.status, TransferStatus::Rejected);
    s.approve(&s.second, "").await.assert_status(422);
    let rows = s.old.units(s.t.db()).await.unwrap();
    assert!(rows[0].date_out.is_none());
    assert!(s.t.cheques.returns().is_empty());
}

/// A clerk who issues invoices.
async fn clerk(t: &Booted) -> User {
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
    t.user_with("Clara", "invoice_clerk").await
}

fn month_of(date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = date.with_day(1).unwrap();
    let next = if start.month() == 12 {
        NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1).unwrap()
    };
    (start, next - Duration::days(1))
}

/// AC1, AC2 with the real wl-invoicing: the invoice issued before the
/// transfer date (the "April" one, paid) keeps its issuer and stays on the old
/// owner's statement; from the transfer date (1 "May") the recreated charge is
/// invoiced from the new owner's entity, and unit 803 is on the new owner's
/// statement only.
#[renox::test]
async fn bl_prp_23_the_issued_invoice_stays_with_the_old_owner_and_the_statements_split() {
    let s = scene_on(boot_with(Real::INVOICING).await, true).await;
    let at = s.at;
    // The new owner's entity.
    let mut second = wl_kernel::LegalEntity {
        code: "PGM".to_owned(),
        name_en: "Palm Grove Management LLC".to_owned(),
        trn: Some("100000000000029".to_owned()),
        vat_registered: true,
        status: "active".to_owned(),
        ..Default::default()
    };
    second.insert(s.t.db()).await.unwrap();
    let buyer = s.buyer_agreement(false).await;
    ManagementAgreement::query()
        .where_(ManagementAgreement::ID.eq(buyer.id))
        .update(s.t.db(), &[("legal_entity_id", &Some(second.id))])
        .await
        .unwrap();
    s.t.post(
        &s.first,
        &format!("/property/agreements/{}/activate", buyer.id),
        &[],
    )
    .await
    .assert_ok();

    // "April": the instalment before the last is invoiced, issued and paid.
    let mut charges = LeaseCharge::of_lease(s.t.db(), s.lease.id).await.unwrap();
    charges.sort_by_key(|c| (c.due_date, c.id));
    let april = charges[charges.len() - 2].clone();
    assert!(april.due_date < at);
    property::invoicing::run(s.t.state(), april.due_date)
        .await
        .unwrap();
    let clerk = clerk(&s.t).await;
    let april = LeaseCharge::find_or_404(s.t.db(), april.id).await.unwrap();
    assert_eq!(april.status, ChargeStatus::Invoiced);
    let april_invoice = april.invoice_id.expect("the April invoice");
    s.t.post(&clerk, &format!("/invoices/{april_invoice}/issue"), &[])
        .await
        .assert_status(303);
    invoicer(s.t.state())
        .allocate(
            s.t.state(),
            InvoiceId(april_invoice),
            Subject::new("payments.payment", april_invoice),
            april.amount,
        )
        .await
        .unwrap();
    let before = invoicer(s.t.state())
        .find(s.t.db(), InvoiceId(april_invoice))
        .await
        .unwrap()
        .expect("the invoice");
    assert_eq!(before.status, InvoiceStatus::Paid);
    let issued = wl_invoicing::Invoice::find_or_404(s.t.db(), april_invoice)
        .await
        .unwrap();
    let (issued_entity, number) = (issued.legal_entity_id, issued.number);
    assert_ne!(issued_entity, Some(second.id));

    // The transfer, requested and approved by another manager.
    s.request(&BUYER.0.to_string()).await.assert_ok();
    s.approve(&s.second, "NEW-OWNER-RCPT").await.assert_ok();

    // April's invoice is untouched: same state, number and issuer.
    let after = wl_invoicing::Invoice::find_or_404(s.t.db(), april_invoice)
        .await
        .unwrap();
    assert_eq!(after.status, InvoiceStatus::Paid);
    assert!(after.number.is_some());
    assert_eq!(after.legal_entity_id, issued_entity);
    assert_eq!(after.number, number);
    let april_charge = LeaseCharge::find_or_404(s.t.db(), april.id).await.unwrap();
    assert_eq!(april_charge.status, ChargeStatus::Invoiced);
    assert_eq!(april_charge.legal_entity_id, None);

    // "May": the recreated charge is invoiced from the new owner's entity.
    let charges = LeaseCharge::of_lease(s.t.db(), s.lease.id).await.unwrap();
    let fresh = charges
        .iter()
        .find(|c| c.legal_entity_id == Some(second.id))
        .expect("the recreated charge")
        .clone();
    property::invoicing::run(s.t.state(), fresh.due_date)
        .await
        .unwrap();
    let fresh = LeaseCharge::find_or_404(s.t.db(), fresh.id).await.unwrap();
    let may_invoice = fresh.invoice_id.expect("the May invoice");
    s.t.post(&clerk, &format!("/invoices/{may_invoice}/issue"), &[])
        .await
        .assert_status(303);
    let may = wl_invoicing::Invoice::find_or_404(s.t.db(), may_invoice)
        .await
        .unwrap();
    assert_eq!(may.status, InvoiceStatus::Issued);
    assert_eq!(may.legal_entity_id, Some(second.id));
    invoicer(s.t.state())
        .allocate(
            s.t.state(),
            InvoiceId(may_invoice),
            Subject::new("payments.payment", may_invoice),
            fresh.amount,
        )
        .await
        .unwrap();

    // The statements. The old owner's April statement has the April invoice
    // and nothing of May; the "May" statements: unit 803 only on the new one.
    let (april_start, april_end) = month_of(april.due_date);
    let (may_start, may_end) = month_of(at);
    let old_april = statement::generate(s.t.state(), s.old.id, april_start, april_end)
        .await
        .unwrap()
        .expect("the old owner's April statement");
    let old_may = statement::generate(s.t.state(), s.old.id, may_start, may_end)
        .await
        .unwrap()
        .expect("the old owner's May statement");
    let new_may = statement::generate(s.t.state(), buyer.id, may_start, may_end)
        .await
        .unwrap()
        .expect("the new owner's May statement");
    let sources = |lines: Vec<StatementLine>| -> Vec<String> {
        lines
            .into_iter()
            .filter(|l| l.kind == LineKind::Collected)
            .filter_map(|l| l.source)
            .collect()
    };
    let april_source = InvoiceId(april_invoice).subject().to_string();
    let may_source = InvoiceId(may_invoice).subject().to_string();
    let on_old_april = sources(old_april.lines(s.t.db()).await.unwrap());
    let on_old_may = sources(old_may.lines(s.t.db()).await.unwrap());
    let on_new_may = sources(new_may.lines(s.t.db()).await.unwrap());
    assert!(on_old_april.contains(&april_source), "{on_old_april:?}");
    assert!(on_old_may.is_empty(), "{on_old_may:?}");
    assert_eq!(on_new_may, [may_source]);
}
