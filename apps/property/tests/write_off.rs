//! BL-PRP-18 (#268): writing off the rent of a managed building. The owner's
//! consent is recorded on the request (the loss is the owner's); the
//! written-off rent appears on the owner statement as "rent written off", a
//! later recovery as "recovered after write-off". A published statement is
//! never edited: the line goes on the next one.

mod support;

use property::agreement::{AgreementStatus, ManagementAgreement};
use property::statement::{self, LineKind, OwnerStatement, StatementLine, StatementStatus};
use property::write_off::{WriteOffConsent, managing_agreement};
use property::{ChargeStatus, Lease, LeaseCharge};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::parties::PartyKind;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, Invoicer, invoicer};
use wl_kernel::{PartyId, Subject, VatCategory};

use support::{ACCOUNTANT, AGENT, Booted, MANAGER, TENANT, aed, boot, day};

/// The owner of the managed unit.
const OWNER: PartyId = PartyId(31);

struct Scene {
    t: Booted,
    agreement: ManagementAgreement,
    lease: Lease,
    accountant: User,
    manager: User,
}

/// A managed unit with an active lease, an accountant and a property manager.
async fn scene() -> Scene {
    let t = boot().await;
    t.parties.add(OWNER.0, "Hessa Al Mansoori", PartyKind::Person);
    let building = t.building().await;
    let unit = t.unit(&building, "1204").await;
    let agent = t.user_in("Ali", AGENT, building.id).await;
    let site_manager = t.user_in("Sami", MANAGER, building.id).await;
    let lease = t.active_lease(&unit, &agent, &site_manager).await;
    let mut agreement = ManagementAgreement {
        owner_party_id: OWNER.0,
        fee_bp: 500,
        payout_frequency: "monthly".to_owned(),
        notice_days: 90,
        start_date: day(-400),
        status: AgreementStatus::Active,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    agreement
        .insert_in_state(&mut tx, AgreementStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    property::agreement::add_unit(t.db(), &agreement, unit.id, day(-400))
        .await
        .unwrap();
    let accountant = t.user_with("Amal", ACCOUNTANT).await;
    let manager = t.user_with("Mona", MANAGER).await;
    Scene {
        t,
        agreement,
        lease,
        accountant,
        manager,
    }
}

impl Scene {
    /// An issued rent invoice of `amount` fils, overdue, nothing paid, as a
    /// charge of the lease. Its id.
    async fn overdue(&self, amount: i64, position: i32) -> i64 {
        let draft = InvoiceDraft::new(TENANT, self.lease.subject())
            .due_date(day(-60))
            .line(DraftLine::new("Rent", amount, VatCategory::Exempt));
        let id = invoicer(self.t.state())
            .draft(self.t.db(), draft)
            .await
            .unwrap()
            .expect("invoicing is on")
            .0;
        self.t.invoices.issue(id);
        self.t.invoices.set_status(id, InvoiceStatus::Overdue);
        LeaseCharge::create(
            self.t.db(),
            LeaseCharge {
                lease_id: self.lease.id,
                position,
                charge_type: "rent".to_owned(),
                description: format!("Rent {position}"),
                due_date: day(-60),
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

    /// The accountant asks to write the invoice off, with the owner's consent
    /// when `consent` is true.
    async fn ask(&self, invoice: i64, consent: bool) -> renox::testing::TestResponse {
        let invoice = invoice.to_string();
        let given_on = day(-1).to_string();
        let mut form = vec![("invoice", invoice.as_str()), ("reason_code", "insolvent")];
        if consent {
            form.push(("consent_note", "E-mail of the owner agreeing"));
            form.push(("consent_by", "Hessa Al Mansoori"));
            form.push(("consent_on", given_on.as_str()));
        }
        self.t
            .post(
                &self.accountant,
                &format!("/property/leases/{}/write-off", self.lease.id),
                &form,
            )
            .await
    }

    /// The lines of the agreement that wait for the next draft.
    async fn waiting(&self) -> Vec<StatementLine> {
        StatementLine::query()
            .where_(StatementLine::AGREEMENT_ID.eq(self.agreement.id))
            .where_(StatementLine::STATEMENT_ID.is_null())
            .order_by(StatementLine::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }

    /// The draft statement of the period around today.
    async fn draft(&self) -> OwnerStatement {
        statement::generate(self.t.state(), self.agreement.id, day(-20), day(10))
            .await
            .unwrap()
            .expect("a draft")
    }
}

/// AC1: a managed building and an overdue rent invoice of AED 6,000: a
/// write-off request without the owner's consent is refused; with the consent
/// attached it goes on (to the approval of a property manager other than the
/// requester, which is wl-invoicing's `FIN.ACL.15`).
#[renox::test]
async fn bl_prp_18_the_owners_consent_is_needed_for_a_managed_building() {
    let s = scene().await;
    let invoice = s.overdue(aed(6_000), 10).await;

    let refused = s.ask(invoice, false).await;
    refused.assert_status(422);
    assert!(refused.text().contains("consent"), "{}", refused.text());
    assert!(s.t.invoices.write_offs().is_empty());
    assert!(WriteOffConsent::of_invoice(s.t.db(), invoice).await.unwrap().is_none());

    s.ask(invoice, true).await.assert_ok();
    let asked = s.t.invoices.write_offs();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].invoice, invoice);
    assert_eq!(asked[0].by, s.accountant.id);
    let consent = WriteOffConsent::of_invoice(s.t.db(), invoice)
        .await
        .unwrap()
        .expect("the consent is kept");
    assert_eq!(consent.agreement_id, s.agreement.id);
    assert_eq!(consent.given_by, "Hessa Al Mansoori");
    assert_eq!(consent.recorded_by, Some(s.accountant.id));

    // Consent without a contact is not a consent.
    let other = s.overdue(aed(1_000), 11).await;
    let form = other.to_string();
    s.t.post(
        &s.accountant,
        &format!("/property/leases/{}/write-off", s.lease.id),
        &[
            ("invoice", &form),
            ("reason_code", "insolvent"),
            ("consent_note", "agreed"),
            ("consent_on", &day(-1).to_string()),
        ],
    )
    .await
    .assert_status(422);
}

/// AC2: on approval the next draft owner statement shows "rent written off
/// -6,000.00".
#[renox::test]
async fn bl_prp_18_the_written_off_rent_is_on_the_next_draft() {
    let s = scene().await;
    let invoice = s.overdue(aed(6_000), 10).await;
    s.ask(invoice, true).await.assert_ok();
    // Nothing yet: the request waits for the approval.
    assert!(s.waiting().await.is_empty());

    s.t.invoices.approve_write_off(s.t.state(), invoice, s.manager.id).await;

    let waiting = s.waiting().await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].kind, LineKind::WrittenOff);
    assert_eq!(waiting[0].amount, -aed(6_000));
    assert!(waiting[0].description.contains("Rent written off"), "{}", waiting[0].description);
    assert_eq!(waiting[0].line_on, wl_kernel::documents::today());
    // It is made once, whatever the events do.
    s.t.invoices.approve_write_off(s.t.state(), invoice, s.manager.id).await;
    assert_eq!(s.waiting().await.len(), 1);

    let draft = s.draft().await;
    assert_eq!(draft.collected, -aed(6_000));
    assert_eq!(draft.balance(), -aed(6_000));
    let lines = draft.lines(s.t.db()).await.unwrap();
    assert!(lines
        .iter()
        .any(|l| l.kind == LineKind::WrittenOff && l.amount == -aed(6_000)));
    assert!(s.waiting().await.is_empty());
}

/// A draft that holds the day gets the line at once; a published statement is
/// never edited, so the line waits for the next draft.
#[renox::test]
async fn bl_prp_18_a_published_statement_is_never_edited() {
    let s = scene().await;
    let first = s.overdue(aed(6_000), 10).await;
    s.ask(first, true).await.assert_ok();

    // A draft of the period exists: the line goes on it.
    let draft = s.draft().await;
    s.t.invoices.approve_write_off(s.t.state(), first, s.manager.id).await;
    let draft = OwnerStatement::find_or_404(s.t.db(), draft.id).await.unwrap();
    assert_eq!(draft.collected, -aed(6_000));
    assert!(s.waiting().await.is_empty());

    // Published: a second write-off waits for the next statement.
    s.t.post_plain(
        &s.manager,
        &format!("/property/statements/{}/transitions/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    s.t.post_plain(
        &s.accountant,
        &format!("/property/statements/{}/transitions/publish", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    let published = OwnerStatement::find_or_404(s.t.db(), draft.id).await.unwrap();
    assert_eq!(published.status, StatementStatus::Published);

    let second = s.overdue(aed(2_500), 11).await;
    s.ask(second, true).await.assert_ok();
    s.t.invoices.approve_write_off(s.t.state(), second, s.manager.id).await;
    let after = OwnerStatement::find_or_404(s.t.db(), draft.id).await.unwrap();
    assert_eq!(after.collected, published.collected);
    assert_eq!(after.lines(s.t.db()).await.unwrap().len(), published.lines(s.t.db()).await.unwrap().len());
    let waiting = s.waiting().await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].amount, -aed(2_500));
}

/// AC3: a later recovery of AED 2,000 shows "recovered after write-off
/// +2,000.00" on the statement of that period.
#[renox::test]
async fn bl_prp_18_a_recovery_is_credited_on_the_statement_of_its_period() {
    let s = scene().await;
    let invoice = s.overdue(aed(6_000), 10).await;
    s.ask(invoice, true).await.assert_ok();
    s.t.invoices.approve_write_off(s.t.state(), invoice, s.manager.id).await;

    s.t.invoices
        .recover(s.t.state(), invoice, aed(2_000), wl_kernel::documents::today())
        .await;
    let waiting = s.waiting().await;
    assert_eq!(waiting.len(), 2);
    let recovered = waiting
        .iter()
        .find(|l| l.kind == LineKind::Recovered)
        .expect("a recovery line");
    assert_eq!(recovered.amount, aed(2_000));
    assert!(recovered.description.contains("Recovered after write-off"), "{}", recovered.description);

    let draft = s.draft().await;
    assert_eq!(draft.collected, -aed(4_000));
    // A second recovery is a second line.
    s.t.invoices
        .recover(s.t.state(), invoice, aed(1_000), wl_kernel::documents::today())
        .await;
    let draft = OwnerStatement::find_or_404(s.t.db(), draft.id).await.unwrap();
    assert_eq!(draft.collected, -aed(3_000));
}

/// AC4: a write-off of the management company's own fee invoice needs no
/// owner consent and is not a rent write-off: no line goes on the statement.
/// An invoice of an unmanaged unit needs none either.
#[renox::test]
async fn bl_prp_18_the_companys_own_invoice_needs_no_consent() {
    let s = scene().await;

    // The fee invoice of the agreement: not an invoice of a lease.
    let fee = invoicer(s.t.state())
        .draft(
            s.t.db(),
            InvoiceDraft::new(OWNER, s.agreement.subject())
                .due_date(day(-60))
                .line(DraftLine::new("Management fee", aed(900), VatCategory::Standard)),
        )
        .await
        .unwrap()
        .expect("invoicing is on")
        .0;
    s.t.invoices.issue(fee);
    assert!(managing_agreement(s.t.db(), fee, day(0)).await.unwrap().is_none());
    s.t.invoices.approve_write_off(s.t.state(), fee, s.manager.id).await;
    assert!(s.waiting().await.is_empty());

    // The rent of a unit nobody manages for an owner.
    let other_building = s.t.building_coded("TWR-B").await;
    let other_unit = s.t.unit(&other_building, "301").await;
    let agent = s.t.user_in("Omar", AGENT, other_building.id).await;
    let site = s.t.user_in("Sara", MANAGER, other_building.id).await;
    let other = s.t.active_lease(&other_unit, &agent, &site).await;
    let draft = InvoiceDraft::new(TENANT, other.subject())
        .due_date(day(-60))
        .line(DraftLine::new("Rent", aed(3_000), VatCategory::Exempt));
    let invoice = invoicer(s.t.state())
        .draft(s.t.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on")
        .0;
    s.t.invoices.issue(invoice);
    LeaseCharge::create(
        s.t.db(),
        LeaseCharge {
            lease_id: other.id,
            position: 10,
            charge_type: "rent".to_owned(),
            description: "Rent 10".to_owned(),
            due_date: day(-60),
            amount: aed(3_000),
            vat_category: VatCategory::Exempt,
            status: ChargeStatus::Invoiced,
            invoice_id: Some(invoice),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let form = invoice.to_string();
    let accountant_there = s.t.user_with("Nadia", ACCOUNTANT).await;
    s.t.post(
        &accountant_there,
        &format!("/property/leases/{}/write-off", other.id),
        &[("invoice", &form), ("reason_code", "insolvent")],
    )
    .await
    .assert_ok();
    assert_eq!(s.t.invoices.write_offs().len(), 1);
    s.t.invoices.approve_write_off(s.t.state(), invoice, s.manager.id).await;
    assert!(s.waiting().await.is_empty());
}

/// An invoice of another lease is not this lease's to write off.
#[renox::test]
async fn bl_prp_18_the_invoice_must_belong_to_the_lease() {
    let s = scene().await;
    let stray = invoicer(s.t.state())
        .draft(
            s.t.db(),
            InvoiceDraft::new(TENANT, Subject::new("property.lease", 999))
                .line(DraftLine::new("Rent", aed(100), VatCategory::Exempt)),
        )
        .await
        .unwrap()
        .expect("invoicing is on")
        .0;
    s.ask(stray, true).await.assert_status(404);
    assert!(s.t.invoices.write_offs().is_empty());
}
