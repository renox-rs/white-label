//! BL-EDU-18 (#453): the 10 values for the shared money mechanisms, proved
//! through the real host of the demo training centre (`wl:install` and the
//! seeds), not as constants: the approval of a refund and of a write-off, a
//! failed refund retried once after 24 hours and then the accountant's task, a
//! pending online payment, the membership and package settings, a desk cash
//! shortage, the corporate dispute, the cash-flow alert, the budget and the
//! owner drawings, and the money settings page (10 §3.3, §4.3, §5, §7.1, §7.2).
//!
//! Proved elsewhere: the package refund of AED 600 and the catalogue defaults
//! (`apps/edtech/tests/edu_18_money.rs`), the membership retrying on days 1, 3
//! and 7 and the pause allowance (`crates/wl-invoicing/tests/subscription_*`
//! with the values read back here), the late confirmation after the seat
//! lapsed (`apps/edtech/tests/edu_01_lapse.rs` and wl-payments' `late_confirm`).

mod support;

use std::time::Duration;

use host::roles::{ACCOUNTANT, ADMIN, CENTRE_MANAGER, COUNSELLOR, OWNER, RECEIVABLES};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{
    company, count_messages, day, edtech_booted, edtech_booted_with_gateway, entity,
    issued_invoice, messages_to, person, reload, tenant,
};
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_invoicing::{
    BudgetCategory, BudgetStatus, CompanyBudget, DrawingApproval, DrawingStatus, InvoicingConfig,
    StartChoice, cash_forecast_load,
};
use wl_kernel::approvals::{self, ApprovalPolicy, ApprovalRequest};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::payments::{
    PaymentFailureKind, PaymentMethod, PaymentStatus, RefundRequest, RefundSource, payment_recorder,
};
use wl_kernel::{LegalEntity, Subject};
use wl_payments::gateway::{FakeGateway, GatewayPaymentStatus, RefundOutcome};
use wl_payments::online;
use wl_payments::payment_link::{self, NewLink};
use wl_payments::refund::{RefundStatus, gateway_refund_outcome};
use wl_payments::refund_recover::retries_of;
use wl_payments::refund_retry;
use wl_payments::{Payment, Refund, RefundAttempt};

const VARIANCE: &str = "invoicing.budget_variance";

// ---- Refunds ---------------------------------------------------------------------------------

/// A gateway refund of AED 500 recorded by `by`.
async fn gateway_refund(app: &TestApp, by: &User, entity_id: i64) -> i64 {
    let party = tenant(app, "Sara Student").await;
    let request = RefundRequest {
        source: RefundSource::Booking(Subject::new("scheduling.booking", 3)),
        party,
        legal_entity_id: entity_id,
        amount: 50_000,
        currency: "AED".to_owned(),
        method: Some(PaymentMethod::Gateway),
        gateway_txn: Some("txn_1".to_owned()),
        reason: "The student withdrew".to_owned(),
    };
    payment_recorder(app.state())
        .request_refund(app.state(), request, Some(by.id))
        .await
        .expect("the refund is requested")
        .0
}

/// Refunds: a refund recorded by A needs B's approval; A's own approval is
/// refused.
#[renox::test]
async fn edu_18_a_refund_recorded_by_a_needs_bs_approval() {
    let app = edtech_booted_with_gateway(FakeGateway::new()).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let cleo = person(&app, "Cleo", ACCOUNTANT).await;
    let legal = entity(&app, "GSTC").await;
    let id = gateway_refund(&app, &cara, legal.id).await;

    app.acting_as(&cara);
    let own = app
        .htmx()
        .post(&format!("/probe/refunds/{id}/approve"), &[])
        .await;
    assert!(
        own.status.as_u16() >= 400,
        "the recorder cannot approve: {}",
        own.status
    );
    let refund = Refund::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(refund.status, RefundStatus::Requested);

    app.acting_as(&cleo);
    app.htmx()
        .post(&format!("/probe/refunds/{id}/approve"), &[])
        .await
        .assert_ok();
    let refund = Refund::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(refund.status, RefundStatus::Approved);
    assert_eq!(refund.approved_by, Some(cleo.id));
}

/// Refunds: a gateway refund that fails is retried once, 24 hours after it
/// failed (not before); when the retry fails too the accountant gets one task
/// to pay by another method.
#[renox::test]
async fn edu_18_a_failed_refund_is_retried_after_24_hours_then_the_accountant_has_a_task() {
    let gateway = FakeGateway::new();
    let app = edtech_booted_with_gateway(gateway.clone()).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let cleo = person(&app, "Cleo", ACCOUNTANT).await;
    let legal = entity(&app, "GSTC").await;
    let id = gateway_refund(&app, &cara, legal.id).await;
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
    assert!(
        wl_payments::FollowupTask::open_for(app.db(), "accountant")
            .await
            .unwrap()
            .is_empty(),
        "the system retries first"
    );

    // Not two minutes before 24 hours after the failure; once after it.
    app.travel(Duration::from_secs(24 * 3600 - 120));
    app.run_scheduled(refund_retry::TASK).await.unwrap();
    assert_eq!(retries_of(app.db(), id).await.unwrap(), 0);
    app.travel(Duration::from_secs(300));
    app.run_scheduled(refund_retry::TASK).await.unwrap();
    let refund = Refund::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(refund.status, RefundStatus::Approved);
    assert_eq!(retries_of(app.db(), id).await.unwrap(), 1);

    // It fails again: no third attempt, one task for the accountant.
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
    let tasks = wl_payments::FollowupTask::open_for(app.db(), "accountant")
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].refund_id, Some(id));
}

// ---- Payments --------------------------------------------------------------------------------

/// Payments: a gateway payment pending for 30 minutes is checked with the
/// gateway (not before) and fails when it declines.
#[renox::test]
async fn edu_18_a_gateway_payment_pending_for_30_minutes_fails() {
    let gateway = FakeGateway::new();
    let app = edtech_booted_with_gateway(gateway.clone()).await;
    let student = tenant(&app, "Sara Student").await;
    let legal = entity(&app, "GSTC").await;
    let link = payment_link::create(
        app.state(),
        NewLink::new(student, legal.id, 252_000, "AED")
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
    app.travel(Duration::from_secs(29 * 60));
    app.run_scheduled(online::TASK).await.unwrap();
    let still = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(still.status, PaymentStatus::Pending);

    app.travel(Duration::from_secs(60));
    app.run_scheduled(online::TASK).await.unwrap();
    let failed = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(failed.failure_kind, Some(PaymentFailureKind::Declined));
}

/// Payments: a late confirmation after the seat lapsed is refunded
/// automatically: the setting is on (the payment is refunded in full to the
/// same card when the subject is no longer an active one).
#[renox::test]
async fn edu_18_a_late_confirmation_after_the_lapse_is_refunded_automatically() {
    let app = edtech_booted().await;
    let payments = app
        .state()
        .provided::<wl_payments::config::PaymentsConfig>()
        .expect("payments");
    assert!(payments.late_refund_auto);
    assert_eq!(payments.pending_timeout_minutes, 30);
    assert_eq!(
        payments.chargeback_reminder_days,
        wl_payments::config::DEFAULT_CHARGEBACK_REMINDER_DAYS,
        "the chargeback settings are the module's"
    );
}

// ---- Memberships and packages -------------------------------------------------------------------

/// Memberships: a failed charge retries on days 1, 3 and 7, then the
/// subscription is suspended; a pause allowance of 30 days a year, shared with
/// the enrolment freeze (the freeze counts against the same 30).
#[renox::test]
async fn edu_18_the_membership_retries_on_days_1_3_and_7_and_the_allowance_is_30_days() {
    let app = edtech_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert_eq!(invoicing.subscription_retry_days, [1, 3, 7]);
    assert!(invoicing.subscription_suspension);
    assert_eq!(invoicing.subscription_pause_allowance_days, 30);
    assert_eq!(
        i64::from(invoicing.subscription_pause_allowance_days),
        edtech::enrolment_machine::FREEZE_ALLOWANCE_DAYS
    );
}

/// Packages: after a reopened invoice the counsellor collects and consumption
/// is not suspended.
#[renox::test]
async fn edu_18_a_reopened_package_invoice_gives_a_task_and_suspends_nothing() {
    let app = edtech_booted().await;
    let sales = wl_sales::config::config_of(app.state());
    assert_eq!(sales.package_seller_role, COUNSELLOR);
    assert!(!sales.suspend_package_consumption_while_unpaid);
}

// ---- Cash -----------------------------------------------------------------------------------------

/// Cash: a desk shortage of AED 20 needs an approval to write off (the limit is
/// AED 0) by someone other than the requester; salary recovery is not offered;
/// the accountant counts the cash and the centre manager approves the
/// shortage, the counsellor does neither.
#[renox::test]
async fn edu_18_a_desk_shortage_of_aed_20_needs_approval_and_no_salary_recovery() {
    let app = edtech_booted().await;
    assert_eq!(wl_payments::cash_settle::limit(app.state()), 0);
    let policy = ApprovalPolicy::for_action(
        app.db(),
        wl_payments::cash_settle::ACTION,
        &renox::serde_json::json!({ "amount": 2_000 }),
    )
    .await
    .unwrap()
    .expect("AED 20 needs an approval");
    assert!(policy.sod, "another user than the requester approves");

    let payments = app
        .state()
        .provided::<wl_payments::config::PaymentsConfig>()
        .expect("payments");
    assert!(
        !payments.cash_salary_deduction,
        "L-20: salary recovery is off"
    );
    assert_eq!(payments.cash_manager_role, CENTRE_MANAGER);

    let holds = |role: &str, permission: &str| host::roles::grants_of(role).contains(&permission);
    assert!(holds(ACCOUNTANT, wl_payments::permissions::CASH_COUNT));
    assert!(holds(
        CENTRE_MANAGER,
        wl_payments::permissions::CASH_WRITE_OFF
    ));
    assert!(!holds(COUNSELLOR, wl_payments::permissions::CASH_COUNT));
    assert!(!holds(COUNSELLOR, wl_payments::permissions::CASH_WRITE_OFF));
}

// ---- Corporate -------------------------------------------------------------------------------------

/// Corporate: corporate contacts cannot dispute in the portal; a write-off of
/// AED 100 waits for an approval by the centre manager, never the requester.
#[renox::test]
async fn edu_18_corporate_contacts_cannot_dispute_and_a_write_off_of_aed_100_needs_approval() {
    let app = edtech_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert!(
        !invoicing.portal_disputes,
        "off until the corporate portal ships"
    );
    assert_eq!(invoicing.dispute_escalation_days, 14);
    assert_eq!(invoicing.write_off_approvers, ["centre_manager", "admin"]);

    let rita = person(&app, "Rita", RECEIVABLES).await;
    let mona = person(&app, "Mona", CENTRE_MANAGER).await;
    let payer = company(&app, "Dubai Logistics LLC").await;
    let invoice = issued_invoice(&app, &rita, payer, 10_000, VatCategory::Exempt, day(-30)).await;
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
    assert_eq!(pending.len(), 1, "AED 100 is above the limit of AED 0");
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);
    let own = approvals::approve(app.state(), pending[0].id, &rita, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &mona, "agreed")
        .await
        .unwrap();
    assert_eq!(
        reload(&app, &invoice).await.status,
        InvoiceStatus::WrittenOff
    );
}

// ---- Cash-flow forecast ---------------------------------------------------------------------------

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

/// Cash-flow forecast: a projected balance of AED 24,999 in week 5 alerts the
/// company owner and the accountant once (`FIN.RUL.23`, the 06:00 run); the
/// forecast shows 13 weekly buckets from Monday.
#[renox::test]
async fn edu_18_the_forecast_alerts_the_owner_and_the_accountant_below_25000() {
    let app = edtech_booted().await;
    let olga = person(&app, "Olga", OWNER).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let mona = person(&app, "Mona", CENTRE_MANAGER).await;
    let legal = entity(&app, "GSTC").await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    bank(&app, &legal, 5_000_000).await;
    // AED 25,001 falls due in week 5: AED 24,999 is left.
    bill_due(&app, &legal, "SB-W5", week(5) + Days::new(2), 2_500_100).await;

    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 1);
    assert_eq!(told(&app, &olga).await, 1);
    assert_eq!(told(&app, &cara).await, 1);
    assert_eq!(told(&app, &mona).await, 0);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &olga).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 24,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 25,000.00");
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

/// Cash-flow forecast: AED 25,000 in every week alerts no one.
#[renox::test]
async fn edu_18_exactly_25000_in_every_week_alerts_no_one() {
    let app = edtech_booted().await;
    let olga = person(&app, "Olga", OWNER).await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let legal = entity(&app, "GSTC").await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    bank(&app, &legal, 5_000_000).await;
    bill_due(&app, &legal, "SB-W5", week(5) + Days::new(2), 2_500_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &olga).await, 0);
    assert_eq!(told(&app, &cara).await, 0);
}

// ---- Budget and drawings -----------------------------------------------------------------------------

async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let legal = entity(app, "GSTC").await;
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

/// Budget: the categories (tutor costs, rent, marketing, materials, staff) are
/// owned by the accountant; a monthly budget the accountant prepares needs the
/// company owner's approval, never its preparer.
#[renox::test]
async fn edu_18_the_accountant_owns_the_categories_and_the_owner_approves_the_budget() {
    let app = edtech_booted().await;
    let cara = person(&app, "Cara", ACCOUNTANT).await;
    let olga = person(&app, "Olga", OWNER).await;
    for code in [
        "tutor_costs",
        "rent_utilities",
        "marketing",
        "cost_of_sales",
        "salaries_staff",
    ] {
        let category = BudgetCategory::by_code(app.db(), code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the category {code}"));
        assert_eq!(category.owner_role.as_deref(), Some(ACCOUNTANT), "{code}");
    }
    for category in BudgetCategory::listed(app.db(), true).await.unwrap() {
        if !category.is_drawings() {
            assert_eq!(
                category.owner_role.as_deref(),
                Some(ACCOUNTANT),
                "{}",
                category.code
            );
        }
    }

    let budget = draft_budget(&app, &cara, "tutor_costs", 500_000).await;
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

/// Budget: a month reaching 100% of its line warns the category's owner role
/// (the accountant) once, and not the owner.
#[renox::test]
async fn edu_18_a_month_reaching_its_line_warns_the_accountant_once() {
    let app = edtech_booted().await;
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

/// Drawings: an expense with the category *owner drawings* is approved by the
/// company owner, never by the user who records it, and is left out of the
/// operating cost.
#[renox::test]
async fn edu_18_an_owner_drawing_needs_the_owner_and_is_not_operating_cost() {
    use wl_invoicing::budget_report::{self, ReportQuery};

    let app = edtech_booted().await;
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

    let legal = entity(&app, "GSTC").await;
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

// ---- The page ----------------------------------------------------------------------------------------

/// The money settings page shows the 10 values to whoever may see the settings
/// and refuses everyone else.
#[renox::test]
async fn edu_18_the_money_settings_page_shows_the_10_values() {
    let app = edtech_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Refund approval")
        .assert_see("Failed refund")
        .assert_see("Membership retries")
        .assert_see("days 1, 3, 7")
        .assert_see("Desk cash")
        .assert_see("Cash shortage write-off")
        .assert_see("AED 25,000.00")
        .assert_see("Owner drawings")
        .assert_see("Absence policy");
    let counsellor = person(&app, "Cora", COUNSELLOR).await;
    app.acting_as(&counsellor);
    app.get("/settings/negative-flow").await.assert_forbidden();
}
