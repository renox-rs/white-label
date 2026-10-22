//! BL-CLN-16 (#364): the 05 values for the shared money mechanisms, proved
//! through the real host of the demo clinic (`wl:install` and the seeds), not
//! as constants: the write-off of a patient balance and of a claim, the
//! patient's billing dispute, a pending online payment, a failed refund, a
//! package bought with a reversed payment, the cash-flow alert, the budget,
//! the owner drawings and the stored approval policies (05 section 3.3, 4.1,
//! 4.2, 5 and 7.2 *Settings for the shared negative flows*).
//!
//! Not proved here, and why: the cash handover (`FIN.CSH.01`, `FIN.ACL.05`,
//! `FIN.ACL.06`, `FIN.ACL.19`) is wl-payments' `BL-PAY-17` (#110), which is not
//! built; the portal complaint form (category *billing*) is a portal page that
//! does not exist, so the dispute is raised through the wl-invoicing service
//! the form will call (`DisputeBy::Party`). Both are in `DEFERRED-b6.md`.

mod support;

use host::roles::{ACCOUNTANT, CLINIC_MANAGER, OWNER, RECEIVABLES, RECEPTIONIST};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use std::time::Duration;
use support::*;
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_invoicing::{
    BudgetCategory, BudgetStatus, CompanyBudget, DrawingApproval, DrawingStatus, InvoicingConfig,
    PayerAgreement, PayerBill, StartChoice, VatRecipient, cash_forecast_load, escalation,
    reminders,
};
use wl_kernel::approvals::{self, ApprovalPolicy, ApprovalRequest, ConditionKind, Step};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DisputeBy, DisputeRequest, InvoiceId, InvoiceStatus, ReopenReason, Unallocate, VatCategory,
    invoicer,
};
use wl_kernel::payer_bills::{
    NewPayerBill, NewPayerBillLine, PayerBillId, PayerBillStatus, payer_bills,
};
use wl_kernel::payments::{
    PaymentFailureKind, PaymentMethod, PaymentStatus, RefundRequest, RefundSource, payment_recorder,
};
use wl_kernel::tickets::task_creator;
use wl_kernel::{LegalEntity, Subject};
use wl_payments::gateway::{FakeGateway, GatewayPaymentStatus, RefundOutcome};
use wl_payments::online;
use wl_payments::payment_link::{self, NewLink};
use wl_payments::refund::{RefundStatus, gateway_refund_outcome};
use wl_payments::refund_recover::retries_of;
use wl_payments::refund_retry;
use wl_payments::{Payment, Refund, RefundAttempt};
use wl_sales::Package;
use wl_sales::PackageStatus;
use wl_sales::notices::PACKAGE_BALANCE_DUE;
use wl_sales::packages::{self, NewPackage};
use wl_ticketing::Ticket;

const VARIANCE: &str = "invoicing.budget_variance";

// ---- invoices and disputes ---------------------------------------------------

/// Done when (invoices and disputes), 1: a write-off of a patient balance,
/// whatever its amount, waits for an approval (the limit is AED 0); the
/// requester never approves it; the owner / medical director does.
#[renox::test]
async fn every_patient_invoice_write_off_creates_an_approval_request() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEIVABLES).await;
    let olga = person(&app, "Olga", OWNER).await;
    let patient = tenant(&app, "Sara Ali").await;

    for fils in [100, 500_000] {
        let invoice =
            issued_invoice(&app, &rita, patient, fils, VatCategory::Exempt, day(-30)).await;
        app.acting_as(&rita);
        app.htmx()
            .post(
                &format!("/invoices/{}/write-off", invoice.id),
                &[("reason_code", "insolvent"), ("reason_text", "")],
            )
            .await
            .assert_ok();
        let pending = approvals::pending_for(app.db(), &invoice.subject())
            .await
            .unwrap();
        assert_eq!(pending.len(), 1, "{fils} fils is above the limit of AED 0");
        assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);
        let own = approvals::approve(app.state(), pending[0].id, &rita, "").await;
        assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
        approvals::approve(app.state(), pending[0].id, &olga, "agreed")
            .await
            .unwrap();
        assert_eq!(
            reload(&app, &invoice).await.status,
            InvoiceStatus::WrittenOff
        );
    }
}

/// The reminders made so far, before and after the due date.
async fn reminders_sent(app: &TestApp) -> usize {
    count_messages(app, reminders::BEFORE_DUE).await
        + count_messages(app, reminders::AFTER_DUE).await
}

/// Done when (invoices and disputes), 2: a patient disputes invoice INV-0042
/// (the call the portal's *billing* complaint form makes, `DisputeBy::Party`):
/// the invoice is `disputed`, a billing ticket is linked and the clinic
/// manager gets it, reminders stop, nobody else's patient may dispute it, and
/// after 14 days the escalation runs (and not on day 13).
#[renox::test]
async fn a_patient_dispute_stops_reminders_and_is_escalated_after_14_days() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEIVABLES).await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let patient = tenant(&app, "Sara Ali").await;
    let other = tenant(&app, "Omar Hassan").await;
    let invoice = issued_invoice(
        &app,
        &rita,
        patient,
        100_000,
        VatCategory::Standard,
        day(10),
    )
    .await;

    let foreign = DisputeRequest::new(
        InvoiceId(invoice.id),
        "not mine",
        DisputeBy::Party { id: other },
    )
    .amount(1_000);
    assert!(matches!(
        dispute::raise(app.state(), foreign).await,
        Err(Error::Forbidden)
    ));
    let own = DisputeRequest::new(
        InvoiceId(invoice.id),
        "I was charged for a service I did not have",
        DisputeBy::Party { id: patient },
    )
    .amount(10_000);
    let id = dispute::raise(app.state(), own).await.unwrap();
    assert!(reload(&app, &invoice).await.disputed);
    let stored = Dispute::find(app.db(), id).await.unwrap().unwrap();
    let ticket = Ticket::find_or_404(app.db(), stored.ticket_id.expect("a ticket"))
        .await
        .unwrap();
    assert_eq!(ticket.assignee_user_id, Some(mona.id));

    // The daily tasks run as the scheduler runs them, day by day: no
    // reminder while the invoice is disputed, and the escalation tells the
    // owner after 14 days and not on day 13.
    let day = 24 * 3600;
    for (travel, label) in [(5, 5), (3, 8), (4, 12)] {
        app.travel(Duration::from_secs(travel * day));
        app.run_scheduled(reminders::TASK).await.unwrap();
        assert_eq!(reminders_sent(&app).await, 0, "no reminder on day {label}");
    }
    app.travel(Duration::from_secs(day));
    app.run_scheduled(escalation::TASK).await.unwrap();
    assert_eq!(
        count_messages(&app, escalation::ESCALATED).await,
        0,
        "not on day 13"
    );
    app.travel(Duration::from_secs(day));
    app.run_scheduled(escalation::TASK).await.unwrap();
    assert!(
        count_messages(&app, escalation::ESCALATED).await >= 1,
        "the invoice's owner is told after 14 days"
    );
    app.travel(Duration::from_secs(6 * day));
    app.run_scheduled(reminders::TASK).await.unwrap();
    assert_eq!(reminders_sent(&app).await, 0, "no reminder on day 20");
}

/// The limits that live on a stored approval policy are the company
/// administrator's to change on the stored row (05 section 7.2): the patient write-off and the
/// payer write-off start at AED 0 for the owner, a dispute credit above the
/// limit is `FIN.ACL.07`, an overpayment or recoupment of a payer is a second
/// person's whatever the amount (`FIN.ACL.16`), a chargeback is accepted by a
/// second person (`FIN.ACL.18`) and an owner drawing is approved whatever its
/// amount (`FIN.ACL.21`).
#[renox::test]
async fn the_limits_are_stored_policies() {
    let app = clinics_booted().await;
    let stored = |action: &'static str| {
        let db = app.db().clone();
        async move {
            let mut rows = ApprovalPolicy::query()
                .where_(ApprovalPolicy::ACTION.eq(action))
                .get(&db)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1, "{action}");
            rows.remove(0)
        }
    };
    let owner = Step::one_of(["owner", "admin"]);

    let write_off = stored("invoicing.write_off").await;
    assert_eq!(write_off.condition, ConditionKind::AmountAbove);
    assert_eq!(write_off.threshold, 0, "FIN.ACL.15: AED 0");
    assert_eq!(write_off.steps.0, std::slice::from_ref(&owner));
    assert!(write_off.sod && write_off.active);

    let payer = stored("invoicing.payer_write_off").await;
    assert_eq!(payer.condition, ConditionKind::AmountAbove);
    assert_eq!(payer.threshold, 0, "the limit is the agreement's");
    assert_eq!(payer.steps.0, std::slice::from_ref(&owner));

    let money = stored("invoicing.payer_money").await;
    assert_eq!(
        (money.condition, money.threshold),
        (ConditionKind::AmountAbove, 0)
    );
    assert!(money.sod, "FIN.ACL.16: a second person, any amount");

    let credit = stored("invoicing.dispute_credit").await;
    assert_eq!(
        (credit.condition, credit.threshold),
        (ConditionKind::AmountAbove, 0)
    );

    let chargeback = stored(wl_payments::chargeback_close::ACCEPT_ACTION).await;
    assert!(chargeback.sod, "FIN.ACL.18: a second person");

    let drawings = stored("invoicing.owner_drawing").await;
    assert_eq!(
        (drawings.condition, drawings.threshold),
        (ConditionKind::AmountAbove, 0)
    );
}

/// A payer write-off policy that is still wl-invoicing's default (a database
/// made before the clinic's seed) is brought to the owner by the invoicing
/// part of the install, whether or not scheduling is on.
#[renox::test]
async fn the_payer_write_off_policy_follows_invoicing_not_scheduling() {
    let app = clinics_booted().await;
    let default = Step::one_of(["admin"]).or_permission("payer_bills.write_off");
    let mut policy = ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("invoicing.payer_write_off"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the policy is stored");
    policy.steps = renox::db::Json(vec![default]);
    policy.save(app.db()).await.unwrap();

    host::clinics_seed::install_for(app.state(), &|module| module == "invoicing")
        .await
        .unwrap();
    let again = ApprovalPolicy::find_or_404(app.db(), policy.id)
        .await
        .unwrap();
    assert_eq!(again.steps.0, [Step::one_of(["owner", "admin"])]);
}

/// The read-only money settings page shows the clinic's own values, not
/// Property's (leases, cheques, deposits).
#[renox::test]
async fn the_money_settings_page_shows_the_05_values_for_the_clinic() {
    let app = clinics_booted().await;
    let admin = person(&app, "Ada", host::roles::ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Money settings")
        .assert_see("Claim write-off limit")
        .assert_see("AED 1,000.00 for a new payer agreement")
        .assert_see("PGC AED 50,000.00")
        .assert_see("checked with the gateway after 30 minutes")
        .assert_see("Pending issue alert")
        .assert_see("owned by accountant")
        .assert_dont_see("Maximum presentations of one cheque")
        .assert_dont_see("Duplicate check keys")
        .assert_dont_see("Block effect on leases");
}

/// `FIN.ACL.07`: a credit that settles a patient's dispute waits for a second
/// person whatever its amount (the limit is AED 0); the accountant who agreed
/// it cannot approve it, another accountant does and the credit follows.
#[renox::test]
async fn a_dispute_credit_waits_for_a_second_person() {
    use wl_invoicing::CreditNote;
    use wl_invoicing::dispute_resolve::{self, ResolveInput};
    use wl_kernel::invoicing::{CreditNoteStatus, DisputeOutcome};

    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEIVABLES).await;
    let cleo = person(&app, "Cleo", RECEIVABLES).await;
    let erin = person(&app, "Erin", ACCOUNTANT).await;
    let patient = tenant(&app, "Sara Ali").await;
    let invoice = issued_invoice(
        &app,
        &rita,
        patient,
        100_000,
        VatCategory::Standard,
        day(10),
    )
    .await;
    let id = dispute::raise(
        app.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "charged for a service I did not have",
            DisputeBy::Party { id: patient },
        )
        .amount(20_000),
    )
    .await
    .unwrap();

    let done = dispute_resolve::resolve(
        app.state(),
        invoice.id,
        ResolveInput::credited("Agreed with the patient", 20_000),
        DisputeBy::User { id: cleo.id },
    )
    .await
    .unwrap();
    let request = done.pending_request.expect("a credit waits for approval");
    let draft = CreditNote::find(app.db(), done.credit_note.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.status, CreditNoteStatus::Draft);
    assert!(reload(&app, &invoice).await.disputed);
    let own = approvals::approve(app.state(), request, &cleo, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approvals::approve(app.state(), request, &erin, "agreed")
        .await
        .unwrap();
    let after = reload(&app, &invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.balance, invoice.balance - 20_000);
    let note = CreditNote::find(app.db(), draft.id).await.unwrap().unwrap();
    assert_eq!(note.status, CreditNoteStatus::Applied);
    let stored = Dispute::find(app.db(), id).await.unwrap().unwrap();
    assert_eq!(stored.outcome, Some(DisputeOutcome::Credited));
}

/// `FIN.ACL.16`: a payer's remittance that pays more than the claim is held
/// for a second person whatever the amount: the accountant who recorded it
/// cannot approve it, another accountant does and only then is the claim paid
/// and the extra kept as the payer's credit.
#[renox::test]
async fn a_payer_overpayment_waits_for_a_second_person() {
    use wl_invoicing::payer_credit;
    use wl_invoicing::payer_remittance::PayerRemittance;

    let app = clinics_booted().await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let cleo = person(&app, "Cleo", ACCOUNTANT).await;
    let patient = tenant(&app, "Sara Ali").await;
    let payer = company(&app, "Daman Insurance").await;
    let legal = entity(&app, "PGC").await;
    let agreement = PayerAgreement::create(
        app.state(),
        PayerAgreement {
            legal_entity_id: legal.id,
            payer_party_id: payer.0,
            name: "Daman Gold".to_owned(),
            active: true,
            vat_recipient: VatRecipient::Customer,
            first_submission_days: 30,
            resubmission_days: 30,
            resubmission_limit: 2,
            ..PayerAgreement::default()
        },
        None,
    )
    .await
    .unwrap();
    let id = payer_bills(app.state())
        .create(
            app.state(),
            NewPayerBill::new(
                agreement.id,
                patient,
                Subject::new("clinics.visit", 8),
                today(),
            )
            .line(NewPayerBillLine::new("Consultation x 1", 40_000)),
            None,
        )
        .await
        .expect("the payer bill is made");
    app.acting_as(&cara);
    app.htmx()
        .post(&format!("/payer-bills/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", id.0));
    let line = wl_invoicing::payer_bill::lines_of(app.db(), id.0)
        .await
        .unwrap()
        .remove(0);

    // AED 450 against a claim of AED 400: held.
    let paid = format!("line_{}_paid", line.id);
    app.htmx()
        .post(
            &format!("/payer-bills/{}/remittance", id.0),
            &[
                ("reference", "ADV-1"),
                ("date", today().to_string().as_str()),
                (paid.as_str(), "450.00"),
            ],
        )
        .await
        .assert_redirect(&format!("/payer-bills/{}", id.0));
    let held = PayerBill::find_or_404(app.db(), id.0).await.unwrap();
    assert_eq!(
        held.status,
        PayerBillStatus::Issued,
        "nothing is recorded yet"
    );
    assert!(
        PayerRemittance::query()
            .get(app.db())
            .await
            .unwrap()
            .is_empty()
    );
    let request = approvals::pending_for(app.db(), &held.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(request.action, "invoicing.payer_money");
    assert_eq!(request.requester_id, cara.id);
    let own = approvals::approve(app.state(), request.id, &cara, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approvals::approve(app.state(), request.id, &cleo, "agreed")
        .await
        .unwrap();
    let paid_bill = PayerBill::find_or_404(app.db(), id.0).await.unwrap();
    assert_eq!(paid_bill.status, PayerBillStatus::Paid);
    assert_eq!((paid_bill.paid_total, paid_bill.balance), (40_000, 0));
    assert_eq!(
        payer_credit::balance(app.db(), payer, legal.id, "AED")
            .await
            .unwrap(),
        5_000,
        "AED 50 is the payer's credit"
    );
}

/// `FIN.ACL.18`: a chargeback is accepted only after a second person approves
/// it: the accountant who asks cannot approve, another accountant does, and
/// only then is the gateway told; a reason is required. A real online payment
/// of an invoice, charged back by the gateway.
#[renox::test]
async fn accepting_a_chargeback_needs_a_second_person_and_tells_the_gateway() {
    use wl_payments::chargeback::{self, DisputeOpened};
    use wl_payments::chargeback_close::ACCEPT_ACTION;

    let gateway = FakeGateway::new();
    let app = clinics_booted_with_gateway(gateway.clone()).await;
    let rita = person(&app, "Rita", RECEIVABLES).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let cleo = person(&app, "Cleo", ACCOUNTANT).await;
    let patient = tenant(&app, "Sara Ali").await;
    let legal = entity(&app, "PGC").await;
    let invoice = issued_invoice(
        &app,
        &rita,
        patient,
        80_000,
        VatCategory::ZeroRated,
        day(10),
    )
    .await;
    let link = payment_link::create(
        app.state(),
        NewLink::new(patient, legal.id, 80_000, "AED")
            .target(&Subject::new("invoicing.invoice", invoice.id), 80_000),
    )
    .await
    .expect("the link is made");
    app.logout();
    app.htmx()
        .post(
            &format!("{}/start", payment_link::path(app.state(), link.id)),
            &[],
        )
        .await
        .assert_status(303);
    let payment = Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("a payment on the link");
    let txn = payment.gateway_transaction.clone().expect("a transaction");
    online::gateway_succeeded(app.state(), &txn, 80_000)
        .await
        .unwrap()
        .expect("confirmed");
    let charged = chargeback::gateway_dispute_opened(
        app.state(),
        &txn,
        &DisputeOpened::new("dp_1").evidence_due(day(30)),
    )
    .await
    .unwrap()
    .expect("charged back");
    assert_eq!(charged.status, PaymentStatus::ChargedBack);

    let path = format!("/payments/{}/accept-chargeback", payment.id);
    app.acting_as(&cara);
    app.htmx()
        .post(&path, &[("reason", " ")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&path, &[("reason", "Not worth contesting")])
        .await
        .assert_ok();
    let waiting = ApprovalRequest::pending_about(app.db(), &payment.subject())
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, ACCEPT_ACTION);
    assert_eq!(waiting[0].requester_id, cara.id);
    assert!(gateway.accepted().is_empty(), "the gateway is not told yet");
    let approve = format!("/approvals/{}/approve", waiting[0].id);
    app.htmx()
        .post(&approve, &[("comment", "")])
        .await
        .assert_forbidden();

    app.acting_as(&cleo);
    app.htmx()
        .post(&approve, &[("comment", "")])
        .await
        .assert_ok();
    let accepted = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(accepted.status, PaymentStatus::Reversed);
    assert_eq!(accepted.dispute_outcome.as_deref(), Some("accepted"));
    assert_eq!(gateway.accepted(), [(txn, "dp_1".to_owned())]);
}

// ---- claims ---------------------------------------------------------------------

/// A draft payer bill of `claimed` fils under `agreement` for visit `visit`,
/// whose service was 100 days ago (the 30-day window of the agreement has
/// passed).
async fn draft_claim(
    app: &TestApp,
    agreement: &PayerAgreement,
    patient: wl_kernel::PartyId,
    visit: i64,
    claimed: i64,
) -> PayerBillId {
    let request = NewPayerBill::new(
        agreement.id,
        patient,
        Subject::new("clinics.visit", visit),
        day(-100),
    )
    .void_before_first_submission()
    .line(NewPayerBillLine::new("Consultation x 1", claimed));
    payer_bills(app.state())
        .create(app.state(), request, None)
        .await
        .expect("the payer bill is made")
}

/// Done when (claims): the claim write-off limit is AED 1,000 from the seed:
/// a payer agreement made from the module's settings starts with it; a
/// write-off of AED 1,000.01 waits for the owner, AED 1,000.00 does not.
#[renox::test]
async fn a_claim_write_off_of_1000_01_needs_the_owner_and_1000_00_does_not() {
    let app = clinics_booted().await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let olga = person(&app, "Olga", OWNER).await;
    let patient = tenant(&app, "Sara Ali").await;
    let payer = company(&app, "Daman Insurance").await;
    let legal = entity(&app, "PGC").await;
    let config = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("the invoicing settings are provided");
    assert_eq!(config.payer_follow_up_days, 45, "FIN.RUL.13: 45 days");
    let made = |name: &str| PayerAgreement {
        legal_entity_id: legal.id,
        payer_party_id: payer.0,
        name: name.to_owned(),
        active: true,
        vat_recipient: VatRecipient::Customer,
        first_submission_days: 30,
        resubmission_days: 30,
        resubmission_limit: 2,
        ..PayerAgreement::default()
    };
    // Made the way any agreement is made, it starts with the clinic's limit,
    // whatever the row said; a limit the caller states is kept.
    let agreement = PayerAgreement::create(app.state(), made("Daman Gold"), None)
        .await
        .unwrap();
    assert_eq!(agreement.write_off_limit, 100_000, "AED 1,000");
    let own = PayerAgreement::create(app.state(), made("Daman Silver"), Some(50_000))
        .await
        .unwrap();
    assert_eq!(own.write_off_limit, 50_000);

    let over = draft_claim(&app, &agreement, patient, 1, 100_001).await;
    app.acting_as(&mona);
    let answer = app
        .htmx()
        .post(
            &format!("/probe/payer-bills/{}/write-off", over.0),
            &[("reason", "missed window")],
        )
        .await;
    answer.assert_ok();
    let text = answer.text();
    let request: i64 = text
        .trim()
        .strip_prefix("pending:")
        .unwrap_or_else(|| panic!("AED 1,000.01 waits: {text}"))
        .parse()
        .expect("an approval request");
    let bill = PayerBill::find_or_404(app.db(), over.0).await.unwrap();
    assert_eq!(bill.status, PayerBillStatus::Draft);
    let own = approvals::approve(app.state(), request, &mona, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), request, &olga, "agreed")
        .await
        .unwrap();
    let closed = PayerBill::find_or_404(app.db(), over.0).await.unwrap();
    assert_eq!(closed.status, PayerBillStatus::Closed);

    let exact = draft_claim(&app, &agreement, patient, 2, 100_000).await;
    app.acting_as(&mona);
    let answer = app
        .htmx()
        .post(
            &format!("/probe/payer-bills/{}/write-off", exact.0),
            &[("reason", "missed window")],
        )
        .await;
    answer.assert_ok();
    assert_eq!(answer.text().trim(), "done", "AED 1,000.00 needs nobody");
    let closed = PayerBill::find_or_404(app.db(), exact.0).await.unwrap();
    assert_eq!(closed.status, PayerBillStatus::Closed);
}

// ---- payments ---------------------------------------------------------------------

/// Done when (payments), 1: a deposit payment pending 30 minutes is checked
/// with the gateway (not before): the gateway's answer decides, here a
/// decline, rather than a timeout.
#[renox::test]
async fn a_deposit_payment_pending_for_30_minutes_is_checked_with_the_gateway() {
    let gateway = FakeGateway::new();
    let app = clinics_booted_with_gateway(gateway.clone()).await;
    let patient = tenant(&app, "Sara Ali").await;
    let legal = entity(&app, "PGC").await;
    let link = payment_link::create(
        app.state(),
        NewLink::new(patient, legal.id, 20_000, "AED")
            .prepaid(Subject::new("scheduling.booking", 3)),
    )
    .await
    .expect("the link is made");
    app.logout();
    app.htmx()
        .post(
            &format!("{}/start", payment_link::path(app.state(), link.id)),
            &[],
        )
        .await
        .assert_status(303);
    let payment = Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .order_by_desc(Payment::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("a payment on the link");
    assert_eq!(payment.status, PaymentStatus::Pending);
    let txn = payment.gateway_transaction.clone().expect("a transaction");
    gateway.set_status(
        &txn,
        GatewayPaymentStatus::Declined {
            reason: "Card declined".to_owned(),
        },
    );
    // The scheduled check runs every 5 minutes: at 29 minutes it leaves the
    // payment alone, at 30 it asks the gateway.
    app.travel(Duration::from_secs(29 * 60));
    app.run_scheduled(online::TASK).await.unwrap();
    let still = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(still.status, PaymentStatus::Pending);

    app.travel(Duration::from_secs(60));
    app.run_scheduled(online::TASK).await.unwrap();
    let failed = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(
        failed.failure_kind,
        Some(PaymentFailureKind::Declined),
        "the gateway was asked: a decline, not a timeout"
    );
}

/// Done when (payments), late confirmation: the gateway confirms a deposit
/// payment after it failed and after the booking was cancelled; the money is
/// refunded to the same card at once, with no second person (the 05 setting
/// is on), through the real booking, payment link and refund of the host.
#[renox::test]
async fn a_late_confirmation_after_the_booking_was_cancelled_is_refunded_at_once() {
    let gateway = FakeGateway::new();
    let app = clinics_booted_with_gateway(gateway.clone()).await;
    let rita = person(&app, "Rita", RECEPTIONIST).await;
    let doctor = clinic_doctor(&app, "Dr A").await;
    let service = clinic_service(&app, "checkup", 0, &doctor).await;
    let patient = tenant(&app, "Sara Ali").await;
    let legal = entity(&app, "PGC").await;
    let booking = clinic_booking(
        &app,
        patient,
        &service,
        &doctor,
        renox::db::now() + renox::chrono::TimeDelta::hours(5),
    )
    .await;
    let link = payment_link::create(
        app.state(),
        NewLink::new(patient, legal.id, 20_000, "AED").prepaid(booking.subject()),
    )
    .await
    .expect("the link is made");
    app.logout();
    app.htmx()
        .post(
            &format!("{}/start", payment_link::path(app.state(), link.id)),
            &[],
        )
        .await
        .assert_status(303);
    let payment = Payment::query()
        .where_(Payment::PAYMENT_LINK_ID.eq(link.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("a payment on the link");
    let txn = payment.gateway_transaction.clone().expect("a transaction");
    online::gateway_failed(app.state(), &txn, PaymentFailureKind::Timeout, "")
        .await
        .unwrap()
        .expect("the payment failed");

    // The booking is cancelled before the gateway answers.
    app.acting_as(&rita);
    app.htmx()
        .post(
            &format!("/scheduling/bookings/{}/cancel", booking.id),
            &[("reason_code", "equipment_failure")],
        )
        .await
        .assert_status(303);

    let late = online::gateway_succeeded(app.state(), &txn, 20_000)
        .await
        .unwrap()
        .expect("confirmed late");
    assert_eq!(late.status, PaymentStatus::Received);
    let refunds = Refund::query()
        .where_(Refund::SOURCE_KIND.eq("payment"))
        .where_(Refund::SOURCE_ID.eq(payment.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(refunds.len(), 1, "one refund of the late payment");
    assert_eq!(refunds[0].amount, 20_000);
    assert_eq!(refunds[0].reason, wl_payments::late::REASON);
    assert_eq!(refunds[0].requested_by, None, "no person asked");
    assert_eq!(refunds[0].approved_by, None, "and none approved");
    assert_eq!(gateway.refunded(), [(txn, 20_000)], "back to the same card");
}

/// Done when (payments), 2: a failed refund is retried once, 24 hours after
/// it failed (not before), and not a second time.
#[renox::test]
async fn a_failed_refund_is_retried_once_after_24_hours() {
    let gateway = FakeGateway::new();
    let app = clinics_booted_with_gateway(gateway.clone()).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let cleo = person(&app, "Cleo", ACCOUNTANT).await;
    let patient = tenant(&app, "Sara Ali").await;
    let legal = entity(&app, "PGC").await;
    let request = RefundRequest {
        source: RefundSource::Booking(Subject::new("scheduling.booking", 3)),
        party: patient,
        legal_entity_id: legal.id,
        amount: 50_000,
        currency: "AED".to_owned(),
        method: Some(PaymentMethod::Gateway),
        gateway_txn: Some("txn_1".to_owned()),
        reason: "The clinic cancelled the booking".to_owned(),
    };
    let id = payment_recorder(app.state())
        .request_refund(app.state(), request, Some(cara.id))
        .await
        .expect("the refund is requested")
        .0;
    app.acting_as(&cleo);
    app.htmx()
        .post(&format!("/probe/refunds/{id}/approve"), &[])
        .await
        .assert_ok();
    app.acting_as(&cara);
    let sent = app
        .htmx()
        .post(&format!("/probe/refunds/{id}/send"), &[])
        .await;
    sent.assert_ok();
    let reference = sent.text().trim().to_owned();
    let failed = gateway_refund_outcome(
        app.state(),
        &reference,
        RefundOutcome::Declined {
            reason: "card expired".to_owned(),
        },
    )
    .await
    .unwrap()
    .expect("the outcome is applied");
    assert_eq!(failed.status, RefundStatus::Failed);

    // The scheduled retry (every 15 minutes): not 1 minute before 24 hours
    // after the failure, once after it.
    app.travel(Duration::from_secs(24 * 3600 - 120));
    app.run_scheduled(refund_retry::TASK).await.unwrap();
    assert!(retries_of(app.db(), id).await.unwrap() == 0);
    let refund = Refund::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(refund.status, RefundStatus::Failed);

    app.travel(Duration::from_secs(300));
    app.run_scheduled(refund_retry::TASK).await.unwrap();
    let refund = Refund::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(refund.status, RefundStatus::Approved);
    assert_eq!(retries_of(app.db(), id).await.unwrap(), 1);
    assert_eq!(gateway.refunded().len(), 2, "the gateway was asked twice");

    // It fails again: no third attempt.
    let second = RefundAttempt::latest(app.db(), id)
        .await
        .unwrap()
        .expect("an attempt")
        .gateway_ref
        .expect("a gateway reference");
    gateway_refund_outcome(
        app.state(),
        &second,
        RefundOutcome::Declined {
            reason: "card closed".to_owned(),
        },
    )
    .await
    .unwrap();
    app.travel(Duration::from_secs(200 * 3600));
    app.run_scheduled(refund_retry::TASK).await.unwrap();
    assert_eq!(
        retries_of(app.db(), id).await.unwrap(),
        1,
        "no third attempt"
    );
    assert_eq!(
        RefundAttempt::of_refund(app.db(), id).await.unwrap().len(),
        2
    );
}

/// Done when (payments), 3: a payment reversed on package PKG-7 leaves it
/// `active` and gives the front desk a collect task; consumption is not
/// suspended and the patient is told the balance is due.
#[renox::test]
async fn a_reversed_payment_leaves_the_package_active_and_the_front_desk_collects() {
    let app = clinics_booted().await;
    let rita = person(&app, "Rita", RECEIVABLES).await;
    let rhea = person(&app, "Rhea", RECEPTIONIST).await;
    let holder = tenant(&app, "Omar Hassan").await;

    let draft = draft_invoice(
        &app,
        holder,
        Subject::new("sales.package", 1),
        150_000,
        VatCategory::ZeroRated,
        day(0),
    )
    .await;
    app.acting_as(&rita);
    app.htmx()
        .post(&format!("/invoices/{}/issue", draft.id), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", draft.id));
    let package = packages::sell(
        app.state(),
        NewPackage::new(holder, "physio_10", 10)
            .item_name("Physiotherapy, 10 sessions")
            .price(150_000)
            .invoice(InvoiceId(draft.id)),
    )
    .await
    .unwrap();
    assert_eq!(package.id, 1, "the invoice names package 1 as its source");
    let payment = Subject::new("payments.payment", 42);
    invoicer(app.state())
        .allocate(app.state(), InvoiceId(draft.id), payment.clone(), 150_000)
        .await
        .expect("the payment is allocated");
    let active = Package::find_or_404(app.db(), package.id).await.unwrap();
    assert_eq!(active.status, PackageStatus::Active);
    packages::consume(
        app.state(),
        package.id,
        4,
        Subject::new("scheduling.booking", 1),
    )
    .await
    .unwrap();

    invoicer(app.state())
        .unallocate(
            app.state(),
            Unallocate::new(
                InvoiceId(draft.id),
                payment.clone(),
                150_000,
                "payments.allocation:11",
                ReopenReason::PaymentReversed,
            ),
        )
        .await
        .expect("the invoice is reopened");

    let after = Package::find_or_404(app.db(), package.id).await.unwrap();
    assert_eq!(
        after.status,
        PackageStatus::Active,
        "the package stays active"
    );
    assert_eq!((after.credits_used, after.remaining()), (4, 6));
    assert_eq!(after.reopened_balance, 150_000);
    let key = wl_sales::package_reopen::task_key(package.id, &payment);
    let found = task_creator(app.state())
        .find_open(app.state(), &key)
        .await
        .unwrap()
        .expect("the collect task is open");
    let ticket = Ticket::find_or_404(app.db(), found.id.0).await.unwrap();
    assert_eq!(
        ticket.assignee_user_id,
        Some(rhea.id),
        "the front desk collects"
    );
    assert_eq!(
        messages_to_party(&app, PACKAGE_BALANCE_DUE, holder)
            .await
            .len(),
        1,
        "the patient is told"
    );
    // Consumption is not suspended (suspend consumption: off).
    packages::consume(
        app.state(),
        package.id,
        1,
        Subject::new("scheduling.booking", 2),
    )
    .await
    .expect("the session can still be used");
}

// ---- the cash-flow forecast ---------------------------------------------------------

fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

async fn bank(app: &TestApp, legal: &LegalEntity, fils: i64) {
    cash_balance::enter(
        app.db(),
        None,
        &EnterBalance {
            legal_entity_id: legal.id,
            bank_account: "ENBD".to_owned(),
            amount: fils,
            as_of: today(),
            note: None,
        },
    )
    .await
    .unwrap();
}

async fn bill_due(app: &TestApp, legal: &LegalEntity, number: &str, due: NaiveDate, fils: i64) {
    let mut bill = SupplierBill {
        party_id: company(app, &format!("Vendor {number}")).await.0,
        legal_entity_id: Some(legal.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: day(-1),
        due_date: Some(due),
        currency: "AED".to_owned(),
        gross_total: fils,
        balance: fils,
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, SupplierBillStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn told(app: &TestApp, user: &User) -> usize {
    messages_to(app, BELOW_THRESHOLD, user).await.len()
}

/// Done when (cash-flow forecast and budget), 1: a projected balance of
/// AED 49,999 in week 5 tells the owner / medical director and the
/// accountant once (`FIN.RUL.23`); the forecast shows 13 weekly buckets from
/// Monday.
#[renox::test]
async fn the_forecast_alerts_the_owner_and_the_accountant_below_50000() {
    let app = clinics_booted().await;
    let olga = person(&app, "Olga", OWNER).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let mona = person(&app, "Mona", CLINIC_MANAGER).await;
    let legal = entity(&app, "PGC").await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    bank(&app, &legal, 10_000_000).await;
    // AED 50,001 falls due in week 5: AED 49,999 is left.
    bill_due(&app, &legal, "SB-W5", week(5) + Days::new(2), 5_000_100).await;

    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 1);
    assert_eq!(told(&app, &olga).await, 1);
    assert_eq!(told(&app, &cara).await, 1);
    assert_eq!(told(&app, &mona).await, 0);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &olga).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 49,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 50,000.00");
    assert_eq!(
        cash_alert::run(app.state(), today()).await.unwrap(),
        0,
        "once"
    );

    let config = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("the invoicing settings are provided");
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&legal), today())
            .await
            .unwrap();
    let forecast: Forecast = cash_forecast::assemble(&inputs);
    assert_eq!(forecast.weeks.len(), 13);
    assert_eq!(forecast.weeks[0].start, this_monday());
    assert!(
        forecast
            .weeks
            .iter()
            .all(|w| w.start.weekday() == Weekday::Mon)
    );
    assert_eq!(config.cash_alert_time, "06:00");
    assert!(config.cash_days_late && !config.cash_disputed_counted);
}

/// Done when (cash-flow forecast and budget), 1 (second part): AED 50,000 in
/// every week alerts no one.
#[renox::test]
async fn exactly_50000_in_every_week_alerts_no_one() {
    let app = clinics_booted().await;
    let olga = person(&app, "Olga", OWNER).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let legal = entity(&app, "PGC").await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    bank(&app, &legal, 10_000_000).await;
    bill_due(&app, &legal, "SB-W5", week(5) + Days::new(2), 5_000_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &olga).await, 0);
    assert_eq!(told(&app, &cara).await, 0);
}

// ---- the budget and the owner drawings ---------------------------------------------------

async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let legal = entity(app, "PGC").await;
    let budget = budget_lines::create(
        app.state(),
        NewBudget {
            legal_entity_id: legal.id,
            fiscal_year: i64::from(today().year()),
            name: String::new(),
            start: StartChoice::Empty,
            revision_reason: None,
        },
        Some(by.id),
    )
    .await
    .expect("the draft is made");
    let category = BudgetCategory::by_code(app.db(), code)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the category {code}"));
    let month = budget.months()[today().month0() as usize];
    budget_lines::set_lines(
        app.db(),
        budget.id,
        &[LineInput {
            category_id: category.id,
            month,
            amount,
            note: None,
        }],
    )
    .await
    .expect("the line is set");
    budget
}

async fn pending(app: &TestApp, budget: &CompanyBudget) -> ApprovalRequest {
    let mut pending = approvals::pending_for(app.db(), &budget.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one pending request");
    pending.remove(0)
}

async fn approved_budget(app: &TestApp, by: &User, owner: &User, code: &str, amount: i64) {
    let budget = draft_budget(app, by, code, amount).await;
    app.acting_as(by);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(app, &budget).await;
    approvals::approve(app.state(), request.id, owner, "")
        .await
        .unwrap();
}

/// A category the accountant adds later starts with the clinic's default
/// owner role (the accountant), unless the form names another; owner drawings
/// get none.
#[renox::test]
async fn a_category_added_later_belongs_to_the_accountant() {
    let app = clinics_booted().await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    app.acting_as(&cara);
    app.htmx()
        .post(
            "/budget-categories",
            &[
                ("code", "equipment"),
                ("name_en", "Equipment"),
                ("kind", "expense"),
                ("active", "on"),
            ],
        )
        .await
        .assert_redirect("/budget-categories");
    app.htmx()
        .post(
            "/budget-categories",
            &[
                ("code", "training"),
                ("name_en", "Training"),
                ("kind", "expense"),
                ("owner_role", "clinic_manager"),
                ("active", "on"),
            ],
        )
        .await
        .assert_redirect("/budget-categories");
    let equipment = BudgetCategory::by_code(app.db(), "equipment")
        .await
        .unwrap()
        .expect("saved");
    assert_eq!(equipment.owner_role.as_deref(), Some(ACCOUNTANT));
    let training = BudgetCategory::by_code(app.db(), "training")
        .await
        .unwrap()
        .expect("saved");
    assert_eq!(training.owner_role.as_deref(), Some(CLINIC_MANAGER));
}

/// Done when (cash-flow forecast and budget), 2 (first part): the categories
/// belong to the accountant; a monthly budget the accountant prepares needs
/// approval by the owner / medical director, never its preparer.
#[renox::test]
async fn the_accountant_owns_the_categories_and_the_owner_approves_the_budget() {
    let app = clinics_booted().await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let olga = person(&app, "Olga", OWNER).await;
    let categories = BudgetCategory::listed(app.db(), true).await.unwrap();
    assert!(!categories.is_empty());
    for category in categories.iter().filter(|c| !c.is_drawings()) {
        assert_eq!(
            category.owner_role.as_deref(),
            Some(ACCOUNTANT),
            "{}",
            category.code
        );
    }
    let budget = draft_budget(&app, &cara, "rent_utilities", 500_000).await;
    app.acting_as(&cara);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(&app, &budget).await;
    assert_eq!(request.requester_id, cara.id);
    let approvers = approvals::notify::step_approvers(app.db(), &request)
        .await
        .unwrap();
    assert!(approvers.iter().any(|u| u.id == olga.id));
    assert!(!approvers.iter().any(|u| u.id == cara.id));
    let own = approvals::approve(app.state(), request.id, &cara, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), request.id, &olga, "")
        .await
        .unwrap();
    let approved = CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap();
    assert_eq!(approved.status, BudgetStatus::Approved);
    assert_eq!(approved.approved_by, Some(olga.id));
}

async fn bill(
    app: &TestApp,
    by: &User,
    supplier: &str,
    number: &str,
    price: &str,
    cost_code: &str,
) -> SupplierBill {
    app.acting_as(by);
    app.htmx()
        .post(
            "/supplier-bills",
            &[
                ("supplier", supplier),
                ("supplier_invoice_number", number),
                ("bill_date", day(0).to_string().as_str()),
                ("currency", "AED"),
                ("line_1_description", "Charge"),
                ("line_1_quantity", "1"),
                ("line_1_unit_price", price),
                ("line_1_vat", "standard"),
                ("line_1_cost_code", cost_code),
            ],
        )
        .await
        .assert_status(303);
    let bill = SupplierBill::query()
        .order_by_desc(SupplierBill::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the bill");
    app.htmx()
        .post(&format!("/supplier-bills/{}/submit", bill.id), &[])
        .await
        .assert_status(303);
    SupplierBill::find_or_404(app.db(), bill.id).await.unwrap()
}

async fn approve_bill(app: &TestApp, by: &User, bill: &SupplierBill) {
    app.acting_as(by);
    app.htmx()
        .post(
            &format!("/supplier-bills/{}/transitions/approve", bill.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
}

/// Done when (cash-flow forecast and budget), 2 (third part): a month
/// reaching 100% of its line warns the category's owner role (the
/// accountant) once, and not the owner.
#[renox::test]
async fn a_month_reaching_its_line_warns_the_accountant_once() {
    let app = clinics_booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let olga = person(&app, "Olga", OWNER).await;
    let vendor = company(&app, "Dubai Utilities LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &olga, "rent_utilities", 500_000).await;

    let first = bill(&app, &amal, &code, "UT-1", "4000", "utilities").await;
    approve_bill(&app, &bilal, &first).await;
    assert_eq!(count_messages(&app, VARIANCE).await, 0);
    let second = bill(&app, &amal, &code, "UT-2", "1000", "utilities").await;
    approve_bill(&app, &bilal, &second).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
    assert_eq!(messages_to(&app, VARIANCE, &bilal).await.len(), 1);
    assert!(messages_to(&app, VARIANCE, &olga).await.is_empty());
    let third = bill(&app, &amal, &code, "UT-3", "300", "utilities").await;
    approve_bill(&app, &bilal, &third).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
}

/// Done when (cash-flow forecast and budget), 2 (second part): an expense with
/// the category *owner drawings* is approved by the owner / medical director,
/// never by the user who records it, and is left out of operating cost.
#[renox::test]
async fn an_owner_drawing_needs_the_owner_and_is_not_operating_cost() {
    let app = clinics_booted().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let olga = person(&app, "Olga", OWNER).await;
    let vendor = company(&app, "Villa Decor LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &olga, "rent_utilities", 500_000).await;

    let drawing = bill(&app, &amal, &code, "VD-1", "2000", "owner_drawings").await;
    let ordinary = bill(&app, &amal, &code, "VD-2", "1000", "other").await;
    approve_bill(&app, &amal, &ordinary).await;
    let row = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(row.status, DrawingStatus::Requested);
    let request_id = row.approval_request_id.expect("an approval request");
    let own = approvals::approve(app.state(), request_id, &amal, "").await;
    assert!(own.is_err(), "the requester cannot approve their drawing");
    approvals::approve(app.state(), request_id, &olga, "")
        .await
        .unwrap();
    let approved = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approved.status, DrawingStatus::Approved);
    assert_eq!(approved.approved_by, Some(olga.id));
    approve_bill(&app, &amal, &drawing).await;

    let legal = entity(&app, "PGC").await;
    let config = wl_invoicing::WlInvoicing::new().config().clone();
    let report = budget_report::build(
        app.db(),
        &config,
        &ReportQuery {
            entities: vec![legal.id],
            month: day(0),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.drawings_total().1, 210_000);
    assert_eq!(report.expense_total().1, 100_000);
}

// ---- the settings, read back from the booted host ------------------------------------------

/// The 05 values as the booted host holds them: the invoicing, payments,
/// sales and inventory settings and the negative stock policy of every store
/// (no `allow_consumption`).
#[renox::test]
async fn the_booted_host_holds_the_05_money_and_stock_values() {
    let app = clinics_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert_eq!(invoicing.write_off_approvers, ["owner", "admin"]);
    assert!(invoicing.portal_disputes);
    assert_eq!(invoicing.dispute_escalation_days, 14);
    assert_eq!(invoicing.payer_write_off_limit, 100_000);
    assert_eq!(invoicing.payer_follow_up_days, 45);
    assert_eq!(invoicing.cash_threshold_of("PGC"), 5_000_000);
    assert_eq!(invoicing.cash_forecast_weeks, 13);
    assert_eq!(invoicing.cash_alert_roles, ["owner", "accountant"]);
    assert_eq!(invoicing.budget_variance_percent, 100);
    assert_eq!(invoicing.budget_variance_role, "accountant");
    assert!(!invoicing.drawings_self_approve);

    let payments = app
        .state()
        .provided::<wl_payments::config::PaymentsConfig>()
        .expect("payments");
    assert_eq!(payments.pending_timeout_minutes, 30);
    assert!(payments.late_refund_auto, "a late confirmation is refunded");
    assert_eq!(
        (payments.refund_retry_hours, payments.refund_max_retries),
        (24, 1)
    );
    assert_eq!(
        payments.chargeback_reminder_days,
        wl_payments::config::DEFAULT_CHARGEBACK_REMINDER_DAYS,
        "05 gives no number of days: the module's default"
    );

    let sales = wl_sales::config::config_of(app.state());
    assert_eq!(sales.package_seller_role, "receptionist");
    assert!(!sales.suspend_package_consumption_while_unpaid);

    let inventory = app
        .state()
        .provided::<wl_inventory::InventoryConfig>()
        .expect("inventory");
    assert_eq!(inventory.pending_alert_hours, 24);
    assert!(inventory.release_needs_second_person("recall"));
    assert_eq!(inventory.reminder_hours("quality", None), 24);

    // 05 §7.2: no branch store allows consumption below zero. A new location
    // blocks (the module's default is pinned here, so that a change of it
    // cannot silently let a clinic store go negative), and so does every
    // location of the booted clinic.
    use wl_inventory::{Location, NegativeStockPolicy};
    assert_eq!(
        Location::new("NEW", "New store").negative_stock_policy,
        NegativeStockPolicy::Block
    );
    for location in Location::query().get(app.db()).await.unwrap() {
        assert_eq!(
            location.negative_stock_policy,
            NegativeStockPolicy::Block,
            "{}",
            location.code
        );
    }
}

/// The cash values of the 05 table that the handover of `BL-PAY-17` (#110,
/// not built) will read are constants in `clinics::seed`. This proves only
/// the constants: it is **not** coverage of the cash handover (one handover
/// per receptionist, branch and day, counted by someone other than the
/// holder, a shortage written off only with approval), which stays open with
/// #110.
#[test]
fn the_cash_values_for_the_future_handover_are_the_05_constants() {
    assert_eq!(clinics::seed::CASH_HOLDER_MODE, "staff");
    assert_eq!(clinics::seed::CASH_HANDOVER_PERIOD, "day");
    assert_eq!(clinics::seed::CASH_SHORTAGE_WRITE_OFF_LIMIT, 0);
    assert!(
        !std::hint::black_box(clinics::seed::SALARY_RECOVERY),
        "salary deduction stays off while L-20 is open"
    );
}
