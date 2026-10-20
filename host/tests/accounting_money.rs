//! BL-ACC-24 (#348), the money settings of the 04 Accounting client run by the
//! real modules of the host: every write-off needs a partner other than the
//! requester (`FIN.ACL.15`), the client disputes and the engagement manager
//! gets the ticket, a bounce adds no fee, the cash-flow alert (`FIN.RUL.23`),
//! the budget and the owner drawings (`FIN.BUD.01`, `FIN.RUL.24`,
//! `FIN.ACL.21`).
//!
//! A disputed cycle invoice leaves the retainer `active` in wl-invoicing
//! itself (`subscription_past_due.rs` there); the credit hold badge of a
//! bounced cheque waits for wl-sales (see `DEFERRED-batch3.md`).

mod support;

use host::roles::{BILLING, ENGAGEMENT_MANAGER, PARTNER};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_invoicing::{
    BudgetCategory, BudgetStatus, CompanyBudget, DrawingApproval, DrawingStatus, StartChoice,
};
use wl_kernel::LegalEntity;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, InvoiceStatus, VatCategory};
use wl_ticketing::Ticket;

const VARIANCE: &str = "invoicing.budget_variance";

fn entities() -> Vec<String> {
    vec!["FLA".to_owned()]
}

/// Done when 3 (first part): a write-off of AED 1 waits for a partner who did
/// not ask; the requester (billing, or a partner) never approves it; the
/// client is sent the write-off notice.
#[renox::test]
async fn every_write_off_needs_a_partner_other_than_the_requester() {
    let app = accounting_booted().await;
    let billing = person(&app, "Bella", BILLING).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let second = person(&app, "Paul", PARTNER).await;
    let client = company(&app, "Gulf Trading A LLC").await;

    let invoice = issued_invoice(&app, &billing, client, 100, VatCategory::Exempt, day(-30)).await;
    app.acting_as(&billing);
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
    assert_eq!(pending.len(), 1, "AED 1 is above the limit of AED 0");
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);
    let own = approvals::approve(app.state(), pending[0].id, &billing, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &partner, "agreed")
        .await
        .unwrap()
        .expect("the approval runs");
    assert_eq!(
        reload(&app, &invoice).await.status,
        InvoiceStatus::WrittenOff
    );
    app.run_all_jobs().await;
    assert!(
        reload(&app, &invoice).await.write_off_notice_on.is_some(),
        "the client got the write-off notice"
    );

    // A partner who asks cannot approve their own request; another can.
    let other = issued_invoice(&app, &billing, client, 100, VatCategory::Exempt, day(-30)).await;
    app.acting_as(&partner);
    app.htmx()
        .post(
            &format!("/invoices/{}/write-off", other.id),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let pending = approvals::pending_for(app.db(), &other.subject())
        .await
        .unwrap();
    let own = approvals::approve(app.state(), pending[0].id, &partner, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &second, "")
        .await
        .unwrap();
    assert_eq!(reload(&app, &other).await.status, InvoiceStatus::WrittenOff);
}

/// Done when 3 (second part): the client disputes its invoice; the engagement
/// manager gets the ticket.
#[renox::test]
async fn the_client_disputes_and_the_engagement_manager_gets_the_ticket() {
    let app = accounting_booted().await;
    let billing = person(&app, "Bella", BILLING).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let other = company(&app, "Gulf Trading B LLC").await;
    let invoice = issued_invoice(
        &app,
        &billing,
        client,
        100_000,
        VatCategory::Standard,
        day(10),
    )
    .await;

    let foreign = DisputeRequest::new(
        InvoiceId(invoice.id),
        "not ours",
        DisputeBy::Party { id: other },
    )
    .amount(1_000);
    assert!(matches!(
        dispute::raise(app.state(), foreign).await,
        Err(Error::Forbidden)
    ));
    let own = DisputeRequest::new(
        InvoiceId(invoice.id),
        "The hours are wrong",
        DisputeBy::Party { id: client },
    )
    .amount(10_000);
    let id = dispute::raise(app.state(), own).await.unwrap();
    assert!(reload(&app, &invoice).await.disputed);
    let stored = Dispute::find(app.db(), id).await.unwrap().unwrap();
    let ticket = Ticket::find_or_404(app.db(), stored.ticket_id.expect("a ticket"))
        .await
        .unwrap();
    assert_eq!(ticket.assignee_user_id, Some(manager.id));
}

/// A bounced cheque adds no fee (none is set) and billing decides.
#[test]
fn a_bounced_cheque_adds_no_fee() {
    let payments = host::payments_for("accounting");
    assert!(payments.config().bounce_fee.is_none());
    assert_eq!(payments.config().cheque_manager_role, BILLING);
}

// ---- the cash-flow forecast -------------------------------------------------

fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

async fn bank(app: &TestApp, entity: &LegalEntity, fils: i64) {
    cash_balance::enter(
        app.db(),
        None,
        &EnterBalance {
            legal_entity_id: entity.id,
            bank_account: "ENBD".to_owned(),
            amount: fils,
            as_of: today(),
            note: None,
        },
    )
    .await
    .unwrap();
}

async fn bill_due(app: &TestApp, entity: &LegalEntity, number: &str, due: NaiveDate, fils: i64) {
    let mut bill = SupplierBill {
        party_id: company(app, &format!("Vendor {number}")).await.0,
        legal_entity_id: Some(entity.id),
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

/// Done when 6: AED 24,999 projected in week 5 alerts the partner and billing
/// once; AED 25,000 in every week alerts no one; 13 weekly buckets from Monday.
#[renox::test]
async fn the_cash_forecast_alerts_the_partner_and_billing_below_25000() {
    let app = accounting_booted().await;
    let partner = person(&app, "Pia", PARTNER).await;
    let billing = person(&app, "Bella", BILLING).await;
    let manager = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let fla = entity(&app, "FLA").await;
    // Any bill of the demo data is not part of the check.
    SupplierBill::query().delete(app.db()).await.unwrap();
    bank(&app, &fla, 5_000_000).await;
    // AED 25,001 falls due in week 5: AED 24,999 is left.
    bill_due(&app, &fla, "SB-W5", week(5) + Days::new(2), 2_500_100).await;

    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 1);
    assert_eq!(told(&app, &partner).await, 1);
    assert_eq!(told(&app, &billing).await, 1);
    assert_eq!(told(&app, &manager).await, 0);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &partner).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 24,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 25,000.00");
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);

    // The forecast: 13 weeks from Monday; the check runs at 06:00 Dubai.
    let module = host::invoicing_for("accounting", &entities());
    let config = module.config().clone();
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&fla), today())
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

#[renox::test]
async fn exactly_25000_in_every_week_alerts_no_one() {
    let app = accounting_booted().await;
    let partner = person(&app, "Pia", PARTNER).await;
    let billing = person(&app, "Bella", BILLING).await;
    let fla = entity(&app, "FLA").await;
    // Any bill of the demo data is not part of the check.
    SupplierBill::query().delete(app.db()).await.unwrap();
    bank(&app, &fla, 5_000_000).await;
    bill_due(&app, &fla, "SB-W5", week(5) + Days::new(2), 2_500_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &partner).await, 0);
    assert_eq!(told(&app, &billing).await, 0);
}

// ---- the budget and the owner drawings -------------------------------------

async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let fla = entity(app, "FLA").await;
    let budget = budget_lines::create(
        app.state(),
        NewBudget {
            legal_entity_id: fla.id,
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

async fn approved_budget(app: &TestApp, by: &User, partner: &User, code: &str, amount: i64) {
    let budget = draft_budget(app, by, code, amount).await;
    app.acting_as(by);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(app, &budget).await;
    approvals::approve(app.state(), request.id, partner, "")
        .await
        .unwrap();
}

/// Done when 7 (first part): the categories belong to billing; a budget the
/// billing role prepares needs a partner other than its preparer.
#[renox::test]
async fn billing_owns_the_categories_and_a_partner_approves_the_budget() {
    let app = accounting_booted().await;
    let billing = person(&app, "Bella", BILLING).await;
    let partner = person(&app, "Pia", PARTNER).await;
    for code in [
        "rent_utilities",
        "professional_fees",
        "bank_charges",
        "sales_services",
    ] {
        let category = BudgetCategory::by_code(app.db(), code)
            .await
            .unwrap()
            .expect(code);
        assert_eq!(category.owner_role.as_deref(), Some(BILLING), "{code}");
    }
    let budget = draft_budget(&app, &billing, "rent_utilities", 500_000).await;
    app.acting_as(&billing);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(&app, &budget).await;
    assert_eq!(request.requester_id, billing.id);
    let approvers = approvals::notify::step_approvers(app.db(), &request)
        .await
        .unwrap();
    assert!(approvers.iter().any(|u| u.id == partner.id));
    assert!(!approvers.iter().any(|u| u.id == billing.id));
    let own = approvals::approve(app.state(), request.id, &billing, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), request.id, &partner, "")
        .await
        .unwrap();
    let approved = CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap();
    assert_eq!(approved.status, BudgetStatus::Approved);
    assert_eq!(approved.approved_by, Some(partner.id));
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

/// Done when 7 (second part): a month reaching 100% of its line warns the
/// category's owner role (billing) once.
#[renox::test]
async fn a_month_reaching_its_line_warns_billing_once() {
    let app = accounting_booted().await;
    let amal = person(&app, "Amal", BILLING).await;
    let bilal = person(&app, "Bilal", BILLING).await;
    let finance = person(&app, "Mona", ENGAGEMENT_MANAGER).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let vendor = company(&app, "Dubai Utilities LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &partner, "rent_utilities", 500_000).await;

    let first = bill(&app, &amal, &code, "UT-1", "4000", "utilities").await;
    approve_bill(&app, &bilal, &first).await;
    assert_eq!(count_messages(&app, VARIANCE).await, 0);
    let second = bill(&app, &amal, &code, "UT-2", "1000", "utilities").await;
    approve_bill(&app, &bilal, &second).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
    assert_eq!(messages_to(&app, VARIANCE, &bilal).await.len(), 1);
    assert!(messages_to(&app, VARIANCE, &finance).await.is_empty());
    let third = bill(&app, &amal, &code, "UT-3", "300", "utilities").await;
    approve_bill(&app, &bilal, &third).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
}

/// Done when 7 (third part): a drawing is approved by a partner other than the
/// user who records it and is left out of operating cost.
#[renox::test]
async fn an_owner_drawing_needs_a_partner_and_is_not_operating_cost() {
    let app = accounting_booted().await;
    let amal = person(&app, "Amal", BILLING).await;
    let partner = person(&app, "Pia", PARTNER).await;
    let vendor = company(&app, "Villa Decor LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &partner, "rent_utilities", 500_000).await;

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
    approvals::approve(app.state(), request_id, &partner, "")
        .await
        .unwrap();
    let approved = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approved.status, DrawingStatus::Approved);
    assert_eq!(approved.approved_by, Some(partner.id));
    approve_bill(&app, &amal, &drawing).await;

    let fla = entity(&app, "FLA").await;
    let config = wl_invoicing::WlInvoicing::new().config().clone();
    let report = budget_report::build(
        app.db(),
        &config,
        &ReportQuery {
            entities: vec![fla.id],
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
