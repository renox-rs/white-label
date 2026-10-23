//! BL-FNB-16 (#380), 06 §3.3, §4.2, §4.6, §5, §7.2, `X-06`, `X-07`, `X-11`,
//! `X-12`, `X-14`, `D-12`, `L-20`: the 06 deployment's values for the shared
//! delivery, sales, payment, cash, cash-flow and budget mechanisms are in
//! force in a fresh 06 deployment (`wl:install` and the seeds), read from the
//! real modules (their settings services, approval policies and jobs), not
//! from constants. A test names the Done-when point it covers; the mechanisms
//! themselves (a COD difference, a refund, a handover, the payment windows, the
//! budget) are run end to end under the same values by `fnb_flows.rs`,
//! `fnb_budget.rs` and the tests of `apps/fnb` and of the modules.

mod support;

use renox::state_machine::StateMachine;

use host::fnb_seed::{self, ORDER_SOURCE};
use host::roles::{
    ACCOUNTANT, ADMIN, AREA_MANAGER, CASHIER, DRIVER, KITCHEN_MANAGER, MEMBER, OPERATIONS_DIRECTOR,
};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_delivery::reasons::FailureReason;
use wl_delivery::{PerishableAction, settings};
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_invoicing::{BudgetCategory, InvoicingConfig};
use wl_kernel::LegalEntity;
use wl_kernel::approvals::ApprovalPolicy;
use wl_kernel::documents::today;
use wl_payments::config::PaymentsConfig;

use support::{company, entity, fnb_booted, messages_to, person};

fn holds(role: &str, permission: &str) -> bool {
    host::roles::grants_of(role).contains(&permission)
}

fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

/// The approval policy of `action` for `amount` fils.
async fn policy_at(app: &TestApp, action: &str, amount: i64) -> Option<ApprovalPolicy> {
    ApprovalPolicy::for_action(
        app.db(),
        action,
        &renox::serde_json::json!({ "amount": amount }),
    )
    .await
    .unwrap()
}

/// AC (Delivery 1): a failed F&B delivery is never queued for reattempt; with
/// the rider able to return it ends `returned`, after a run abort it ends
/// `discarded` (with a photo); the failure reasons include *rider issue* and
/// the two causes of *refused*.
#[renox::test]
async fn a_failed_delivery_returns_or_discards_and_is_never_reattempted() {
    let app = fnb_booted().await;
    let delivery = settings::settings(app.state());
    let policy = delivery.perishable_policy(ORDER_SOURCE);
    assert_eq!(policy.on_failed_visit, PerishableAction::Return);
    assert_eq!(policy.on_abort, PerishableAction::Discard);
    assert!(
        !delivery.attempt_rule(),
        "no reattempt rule for cooked food"
    );
    let reasons = FailureReason::active_ones(app.db()).await.unwrap();
    for code in ["rider_issue", "refused_late", "refused_wrong_item"] {
        let reason = reasons
            .iter()
            .find(|r| r.code == code)
            .unwrap_or_else(|| panic!("the failure reason {code}"));
        assert!(!reason.allows_reattempt, "{code} is never reattempted");
        assert_eq!(reason.counts, code != "rider_issue", "{code}");
    }
}

/// AC (Delivery 2): a COD order of AED 80.00 paid AED 76.00 needs no approval
/// (a difference of AED 4.00 is within the AED 5.00 tolerance), paid AED 74.00
/// needs the kitchen manager (AED 6.00 is above it); the rider cannot approve.
#[renox::test]
async fn a_cod_difference_above_aed_5_needs_the_kitchen_manager() {
    let app = fnb_booted().await;
    let delivery = settings::settings(app.state());
    assert_eq!(delivery.cod_tolerance(), 500);
    assert!(8_000 - 7_600 <= delivery.cod_tolerance());
    assert!(8_000 - 7_400 > delivery.cod_tolerance());
    assert!(delivery.cod_approval());
    assert!(holds(
        KITCHEN_MANAGER,
        wl_delivery::permissions::APPROVE_COD_DIFFERENCE
    ));
    assert!(!holds(
        DRIVER,
        wl_delivery::permissions::APPROVE_COD_DIFFERENCE
    ));
}

/// AC (Delivery 3): partial acceptance is on for the F&B order; a line of up
/// to AED 50.00 refused at the door is refunded at once, above it a kitchen
/// manager files the refund; the promise is 45 minutes; the lost mark comes
/// after 7 days.
#[renox::test]
async fn the_customer_may_refuse_a_line_at_the_door() {
    let app = fnb_booted().await;
    let delivery = settings::settings(app.state());
    assert!(delivery.partial_acceptance(ORDER_SOURCE));
    assert_eq!(delivery.promise_minutes(ORDER_SOURCE), Some(45));
    assert_eq!(delivery.untraced_days(), 7);
    let fnb_settings = app
        .state()
        .provided::<fnb::settings::FnbSettings>()
        .unwrap();
    assert_eq!(fnb_settings.refused_refund_limit, 5_000);
    // A Burger of AED 40.00 is within it, a basket above AED 50.00 is not.
    assert!(4_000 <= fnb_settings.refused_refund_limit);
    assert!(5_001 > fnb_settings.refused_refund_limit);
}

/// AC (Delivery 4 / reversal): a reversal after delivery and a manual lost
/// mark are approved by the kitchen manager (or the company admin), never the
/// requester, and the rider neither requests nor approves.
#[renox::test]
async fn a_reversal_and_a_lost_mark_are_approved_by_the_kitchen_manager() {
    let app = fnb_booted().await;
    for action in [wl_delivery::reverse::ACTION, wl_delivery::lost::ACTION] {
        let policy = policy_at(&app, action, 0)
            .await
            .unwrap_or_else(|| panic!("the policy {action}"));
        assert!(policy.sod, "{action}");
        let step = &policy.steps.0[0];
        assert!(step.allows(KITCHEN_MANAGER), "{action}");
        assert!(step.allows(ADMIN), "{action}");
        assert!(!step.allows(DRIVER), "{action}");
    }
    assert!(!holds(DRIVER, wl_delivery::permissions::REVERSE));
    assert!(holds(KITCHEN_MANAGER, wl_delivery::permissions::REVERSE));
}

/// AC (Sales 1, 3): an aggregator order offers no amend and a second order
/// with the same aggregator number is refused (the setting is on); cancel
/// after the kitchen started offers only *return to stock* and *scrap*.
#[renox::test]
async fn the_sales_settings_are_the_06_values() {
    let app = fnb_booted().await;
    let sales = wl_sales::config::config_of(app.state());
    for channel in fnb_seed::AGGREGATOR_CHANNELS {
        assert!(!sales.amendments_allowed(channel), "{channel}");
    }
    assert!(sales.amendments_allowed("website"));
    assert!(sales.unique_external_order_number);
    assert_eq!(sales.cancel_outcomes, ["return_to_stock", "scrap"]);
    assert!(sales.offers_outcome("scrap"));
    assert!(!sales.offers_outcome("finish_to_stock"));
    assert_eq!(
        (sales.cancellation_fee_bps, sales.cancellation_fee_amount),
        (0, 0)
    );
    assert_eq!(sales.cancel_approval_limit, 5_000);
    assert_eq!(sales.online_payment_window_minutes, 15);
}

/// AC (Sales 2): a *no goods* refund of AED 51.00 needs the kitchen manager;
/// the recorder (the cashier) cannot approve.
#[renox::test]
async fn a_no_goods_refund_above_aed_50_needs_the_kitchen_manager() {
    let app = fnb_booted().await;
    let sales = wl_sales::config::config_of(app.state());
    assert_eq!(sales.refund_approval_limit, 5_000);
    assert!(5_100 > sales.refund_approval_limit);
    assert!(holds(
        KITCHEN_MANAGER,
        wl_sales::permissions::RETURNS_APPROVE
    ));
    assert!(!holds(CASHIER, wl_sales::permissions::RETURNS_APPROVE));
    assert!(holds(CASHIER, wl_sales::permissions::RETURNS_REQUEST));
    assert!(sales.complaint_categories.iter().any(|c| c == "wrong_item"));
    assert!(sales.responsible_parties.iter().any(|p| p == "business"));
}

/// AC (Payments 1): a D2C payment pending 10 minutes is checked with the
/// gateway; one still pending at 15 minutes is `payment_failed`; a payment
/// confirmed after a rejection is refunded without staff action; a failed
/// refund is retried once after 24 hours.
#[renox::test]
async fn the_online_payment_values_are_the_06_ones() {
    let app = fnb_booted().await;
    let payments = app.state().provided::<PaymentsConfig>().unwrap();
    assert_eq!(payments.pending_timeout_minutes, 10);
    assert!(payments.late_refund_auto);
    assert_eq!(payments.refund_retry_hours, 24);
    assert_eq!(payments.refund_max_retries, 1);
    let sales = wl_sales::config::config_of(app.state());
    assert_eq!(sales.online_payment_window_minutes, 15);
}

/// AC (Payments 2): a chargeback cannot be accepted by the person who opened
/// it: a second person decides, always (AED 0), and the accountant is the
/// holder.
#[renox::test]
async fn a_chargeback_cannot_be_accepted_by_the_person_who_opened_it() {
    let app = fnb_booted().await;
    for amount in [0, 1, 1_000_000] {
        let policy = policy_at(&app, wl_payments::chargeback_close::ACCEPT_ACTION, amount)
            .await
            .expect("every acceptance needs a second person");
        assert!(policy.sod, "the opener never approves");
        assert_eq!(
            policy.steps.0[0].permission.as_deref(),
            Some(wl_payments::permissions::CHARGEBACK)
        );
    }
    assert!(holds(ACCOUNTANT, wl_payments::permissions::CHARGEBACK));
    assert!(!holds(
        KITCHEN_MANAGER,
        wl_payments::permissions::CHARGEBACK
    ));
}

/// AC (Cash 1): a rider's handover cannot be counted by the rider (the
/// kitchen manager and the cashier count, the rider holds no count
/// permission); a AED 15.00 shortage is written off without approval, AED 25.00
/// needs the area manager; salary recovery is not offered.
#[renox::test]
async fn a_shortage_up_to_aed_20_is_written_off_and_above_it_the_area_manager_approves() {
    let app = fnb_booted().await;
    assert_eq!(wl_payments::cash_settle::limit(app.state()), 2_000);
    assert!(
        policy_at(&app, wl_payments::cash_settle::ACTION, 1_500)
            .await
            .is_none()
    );
    assert!(
        policy_at(&app, wl_payments::cash_settle::ACTION, 2_000)
            .await
            .is_none()
    );
    let policy = policy_at(&app, wl_payments::cash_settle::ACTION, 2_500)
        .await
        .expect("AED 25.00 needs an approval");
    assert!(policy.sod);
    let step = &policy.steps.0[0];
    assert!(step.allows(AREA_MANAGER));
    assert!(step.allows(ADMIN));
    assert!(
        !step.allows(KITCHEN_MANAGER),
        "the one who asks cannot approve"
    );
    assert!(!step.allows(CASHIER));

    let count = wl_payments::permissions::CASH_COUNT;
    assert!(holds(KITCHEN_MANAGER, count));
    assert!(!holds(DRIVER, count), "a rider never counts");
    // The cashier counts through the extra grants of the F&B deployment.
    assert!(
        host::roles::grants_of(CASHIER).contains(&count),
        "the cashier counts"
    );
    assert!(holds(
        KITCHEN_MANAGER,
        wl_payments::permissions::CASH_WRITE_OFF
    ));
    assert!(holds(
        AREA_MANAGER,
        wl_payments::permissions::CASH_WRITE_OFF
    ));

    let payments = app.state().provided::<PaymentsConfig>().unwrap();
    assert!(
        !payments.cash_salary_deduction,
        "L-20 is open: salary recovery is not offered"
    );
    assert_eq!(payments.cash_manager_role, KITCHEN_MANAGER);
}

/// An entity's bank balance of `fils`, as of today.
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

/// An approved supplier bill of `fils` due `due`, payable by the entity.
async fn bill_due(app: &TestApp, entity: &LegalEntity, number: &str, due: NaiveDate, fils: i64) {
    let mut bill = SupplierBill {
        party_id: company(app, &format!("Vendor {number}")).await.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: today() - Days::new(1),
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

/// A fresh 06 deployment with the operations director and the accountant.
async fn scene() -> (TestApp, LegalEntity, User, User) {
    let app = fnb_booted().await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    let zcd = entity(&app, "ZCD").await;
    // The Sharjah entity has cash of its own, so that it is not below its
    // threshold and only ZCD is under test.
    let zcs = entity(&app, "ZCS").await;
    bank(&app, &zcs, 10_000_000).await;
    (app, zcd, director, accountant)
}

async fn told(app: &TestApp, user: &User) -> usize {
    messages_to(app, BELOW_THRESHOLD, user).await.len()
}

/// AC (Cash-flow 1): a projected balance of AED 49,999 in week 5 alerts the
/// operations director and the accountant once.
#[renox::test]
async fn forty_nine_thousand_nine_hundred_ninety_nine_in_week_five_alerts_once() {
    let (app, zcd, director, accountant) = scene().await;
    // AED 100,000 in the bank; a bill of AED 50,001 falls due in week 5.
    bank(&app, &zcd, 10_000_000).await;
    bill_due(&app, &zcd, "SB-W5", week(5) + Days::new(2), 5_000_100).await;

    let sent = cash_alert::run(app.state(), today()).await.unwrap();
    assert_eq!(sent, 1);
    assert_eq!(told(&app, &director).await, 1);
    assert_eq!(told(&app, &accountant).await, 1);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &director).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 49,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 50,000.00");
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &director).await, 1);
}

/// AC (Cash-flow 1): AED 50,000 or more in every week alerts no one.
#[renox::test]
async fn fifty_thousand_in_every_week_alerts_no_one() {
    let (app, zcd, director, accountant) = scene().await;
    bank(&app, &zcd, 10_000_000).await;
    bill_due(&app, &zcd, "SB-W5", week(5) + Days::new(2), 5_000_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &director).await, 0);
    assert_eq!(told(&app, &accountant).await, 0);
}

/// AC (Cash-flow 1): the forecast shows 13 weekly buckets from Monday, the
/// daily check runs at 06:00 Asia/Dubai, due dates are not moved by days late
/// and disputed amounts are not counted.
#[renox::test]
async fn the_forecast_has_thirteen_weeks_from_monday_and_runs_at_six() {
    let (app, zcd, _director, _accountant) = scene().await;
    bank(&app, &zcd, 10_000_000).await;
    let config = app.state().provided::<InvoicingConfig>().unwrap();
    assert!(!config.cash_days_late);
    assert!(!config.cash_disputed_counted);
    assert_eq!(config.cash_alert_roles, [OPERATIONS_DIRECTOR, ACCOUNTANT]);
    assert_eq!(config.cash_threshold_of("ZCD"), 5_000_000);
    assert_eq!(config.cash_threshold_of("ZCS"), 5_000_000);
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&zcd), today())
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

    use renox::Module;
    let mut registry = renox::Registry::default();
    host::invoicing_for("fnb", &["ZCD".to_owned()]).register(&mut registry);
    let run = registry
        .schedule()
        .upcoming(renox::timezone::Zone::UTC)
        .into_iter()
        .find(|run| run.name == cash_alert::TASK)
        .expect("the daily cash check is scheduled");
    let at = run.at.expect("it has a next run");
    assert_eq!(run.zone.to_string(), "Asia/Dubai");
    assert_eq!(
        run.zone.local(at.timestamp()).format("%H:%M").to_string(),
        "06:00"
    );
}

/// AC (Cash-flow 2): the accountant prepares the budget and the operations
/// director approves it; a drawing is approved by the operations director,
/// never by the person who recorded it; `FIN.RUL.24` warns the category's
/// owner role (the accountant) and the accountant at 100%; the categories of
/// 06 exist and the accountant owns them.
#[renox::test]
async fn the_budget_is_approved_by_the_operations_director_and_the_accountant_owns_the_categories()
{
    let app = fnb_booted().await;
    let config = app.state().provided::<InvoicingConfig>().unwrap();
    assert!(!config.drawings_self_approve);
    assert_eq!(config.drawings_notify_role, OPERATIONS_DIRECTOR);
    assert_eq!(config.budget_variance_percent, 100);
    assert_eq!(config.budget_variance_role, ACCOUNTANT);
    assert_eq!(config.budget_category_owner.as_deref(), Some(ACCOUNTANT));
    assert!(holds(ACCOUNTANT, wl_invoicing::permissions::BUDGETS_MANAGE));
    assert!(!holds(
        ACCOUNTANT,
        wl_invoicing::permissions::BUDGETS_APPROVE
    ));
    assert!(holds(
        OPERATIONS_DIRECTOR,
        wl_invoicing::permissions::BUDGETS_APPROVE
    ));
    assert!(holds(
        OPERATIONS_DIRECTOR,
        wl_invoicing::permissions::DRAWINGS_APPROVE
    ));
    assert!(!holds(
        ACCOUNTANT,
        wl_invoicing::permissions::DRAWINGS_APPROVE
    ));
    assert!(holds(
        OPERATIONS_DIRECTOR,
        wl_invoicing::permissions::CASHFLOW_VIEW
    ));

    let categories = BudgetCategory::listed(app.db(), true).await.unwrap();
    for category in categories.iter().filter(|c| !c.is_drawings()) {
        assert_eq!(
            category.owner_role.as_deref(),
            Some(ACCOUNTANT),
            "{}",
            category.name_en
        );
    }
    for (code, ..) in fnb_seed::BUDGET_CATEGORIES {
        assert!(
            categories.iter().any(|c| c.code == code),
            "the category {code}"
        );
    }
    // The owner drawings category is kept apart from the operating ones.
    assert!(categories.iter().any(BudgetCategory::is_drawings));
}

/// The settings page: the operations director and the company admin read the
/// 06 rows; a member and a rider do not.
#[renox::test]
async fn the_operations_director_sees_the_06_money_settings_and_a_member_does_not() {
    let app = fnb_booted().await;
    let director = person(&app, "Omar", OPERATIONS_DIRECTOR).await;
    app.acting_as(&director);
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Perishable policy")
        .assert_see("Partial acceptance")
        .assert_see("Shortage write-off without approval")
        .assert_see("AED 20.00")
        .assert_see("L-20")
        .assert_see("AED 50,000.00");
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/negative-flow").await.assert_ok();
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/negative-flow").await.assert_forbidden();
    let rider = person(&app, "Rami", DRIVER).await;
    app.acting_as(&rider);
    app.get("/settings/negative-flow").await.assert_forbidden();
}
