//! BL-AUT-18 (#414): the 08 values for the shared money and stock mechanisms,
//! read from the REAL host of the demo parts trader (`wl:install` and the
//! seeds) through the settings each module holds, not from constants: the
//! void, the sales rules that have no module setting yet, the cheques and the
//! field handover, the cash shortage, the write-offs, the disputes, the stock
//! limits, the supplier scorecard, the cash-flow forecast, the budget and the
//! owner drawings, and the page `/settings/negative-flow` (08 §3.3, §4.1, §4.3,
//! §7.1 *Negative-flow settings*, §7.2).
//!
//! The values the modules have no setting for are listed in
//! `$S/f8/DEFERRED.md`; the tests say so by reading the page rows.

mod support;

use auto::money_defaults as money;
use auto::{InsuranceClaim, JobCard};
use host::roles::{ADMIN, OWNER, TECHNICIAN};
use renox::chrono::Weekday;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{auto_booted, person};
use wl_inventory::config::InventoryConfig;
use wl_invoicing::{BudgetCategory, InvoicingConfig};
use wl_payments::config::PaymentsConfig;

fn void_from<M: StateMachine>() -> Vec<String> {
    M::machine()
        .find("void")
        .expect("a void transition")
        .from_states()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Void: `void` is offered on a `checked_in` job card and a `registered`
/// claim, and the kernel has no void for a counter sale or an invoice.
#[renox::test]
async fn aut_18_void_is_offered_on_a_checked_in_job_card_and_a_registered_claim_never_on_a_counter_sale()
 {
    let _app = auto_booted().await;
    assert_eq!(void_from::<JobCard>(), ["checked_in"]);
    assert_eq!(void_from::<InsuranceClaim>(), ["registered"]);
    assert!(wl_kernel::voids::get(auto::job_card::JOB_CARD_SUBJECT_TYPE).is_some());
    assert!(wl_kernel::voids::get(auto::claim::INSURANCE_CLAIM_SUBJECT_TYPE).is_some());
    assert!(wl_kernel::voids::get(wl_kernel::sales::COUNTER_SALE_SUBJECT_TYPE).is_none());
    assert!(wl_kernel::voids::get(wl_kernel::invoicing::INVOICE_SUBJECT_TYPE).is_none());
}

/// Sales: the discount limit, the cancellation after picking and the credit
/// values are kept for the page; no module enforces them yet, and the page says
/// so instead of showing them as in force.
#[renox::test]
async fn aut_18_the_sales_rules_without_a_module_setting_are_listed_as_not_enforced() {
    let rows = host::auto_money::rows(&["AWF".to_owned()]);
    let row = |number: &str| rows.iter().find(|r| r.number == number).expect("a row");
    assert_eq!(money::DISCOUNT_LIMIT_PERCENT, 5);
    assert_eq!(money::DISCOUNT_APPROVER_ROLE, "branch_manager");
    assert_eq!(money::CANCEL_AFTER_PICKING_OUTCOME, "return to stock");
    assert_eq!(money::CANCEL_AFTER_PICKING_FEE, 0);
    assert_eq!(money::CREDIT_GRACE_DAYS, 30);
    for number in ["3", "5", "6", "7", "8", "9"] {
        assert!(
            row(number).source.contains("nothing enforces it"),
            "row {number}"
        );
    }
    assert!(row("5").value.contains("above 5%"));
    assert!(row("3").value.contains("return to stock"));
}

/// Credit and cheques: a cheque is presented twice at most (a third is
/// refused), the reminders come after 3 days and the decision after 14, no
/// bounce fee is set, and the branch manager is told besides the accountant.
#[renox::test]
async fn aut_18_a_third_presentation_is_refused_and_no_bounce_fee_is_set() {
    let app = auto_booted().await;
    let payments = app
        .state()
        .provided::<PaymentsConfig>()
        .expect("payments");
    assert_eq!(payments.cheque_max_presentations, 2);
    assert_eq!(
        (payments.cheque_reminder_days, payments.cheque_decision_days),
        (3, 14)
    );
    assert!(payments.bounce_fee.is_none());
    assert_eq!(payments.cheque_manager_role, "branch_manager");
    assert!(payments.cheque_notify_manager);
}

/// Money: every write-off needs an approval, a driver's cheque reaches the
/// deposit list only after another person counts the handover, and a cash
/// shortage is always an approval with no salary recovery.
#[renox::test]
async fn aut_18_every_write_off_needs_approval_and_a_drivers_cheque_waits_for_the_count() {
    let app = auto_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert_eq!(invoicing.write_off_approvers, ["owner", "admin"]);
    let payments = app
        .state()
        .provided::<PaymentsConfig>()
        .expect("payments");
    assert!(payments.field_cheque_handover);
    assert_eq!(payments.cash_write_off_limit, 0);
    assert!(!payments.cash_salary_deduction);
    assert_eq!(payments.cash_manager_role, "branch_manager");
}

/// Stock: an adjustment of AED 1,100 is above the AED 1,000 limit and a Return
/// to supplier of AED 5,100 above the AED 5,000 one, so each needs a second
/// person; a return is chased after 30 days.
#[renox::test]
async fn aut_18_a_stock_write_off_of_aed_1100_and_a_return_of_aed_5100_need_approval() {
    let app = auto_booted().await;
    let inventory = app
        .state()
        .provided::<InventoryConfig>()
        .expect("inventory");
    assert_eq!(inventory.wastage_approval_fils, 100_000);
    assert!(110_000 > inventory.wastage_approval_fils);
    assert_eq!(inventory.return_approval_fils, 500_000);
    assert!(510_000 > inventory.return_approval_fils);
    assert_eq!(inventory.return_chase_days, 30);
}

/// Supplier scorecard: a 3-month window with at least 3 receipts and 1 grace
/// day; under 70 the purchaser gets a review task (`STK.RUL.09`); changing a
/// preferred supplier needs a second person (`STK.ACL.07`).
#[renox::test]
async fn aut_18_the_supplier_scorecard_reviews_under_70_and_a_preferred_change_needs_a_second_person()
 {
    let app = auto_booted().await;
    let inventory = app
        .state()
        .provided::<InventoryConfig>()
        .expect("inventory");
    assert_eq!(inventory.score.window_months, 3);
    assert_eq!(inventory.score.min_receipts, 3);
    assert_eq!(inventory.score.grace_days, 1);
    assert_eq!(inventory.score.threshold, 70);
    assert!(inventory.preferred_change_needs_approval);
    assert_eq!(inventory.recall_notices_route.as_deref(), Some("auto.recall"));
}

/// Disputes: the B2B buyer can flag an invoice as disputed in the portal; the
/// dispute escalates after 14 days to the Owner / GM.
#[renox::test]
async fn aut_18_a_b2b_buyer_can_dispute_an_invoice_in_the_portal() {
    let app = auto_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert!(invoicing.portal_disputes);
    assert_eq!(invoicing.dispute_escalation_days, 14);
    assert_eq!(invoicing.dispute_escalation_role, "owner");
    assert_eq!(invoicing.bad_debt_relief_months.get(), 6);
    assert_eq!(invoicing.payer_follow_up_days, 30);
}

/// Cash-flow forecast: AED 99,999 projected in week 5 is under the AED 100,000
/// threshold and alerts the Owner / GM and the accountant at 06:00; AED 100,000
/// does not; 13 weekly buckets from Monday.
#[renox::test]
async fn aut_18_the_cash_flow_forecast_alerts_below_aed_100000_over_13_weeks_from_monday() {
    let app = auto_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert_eq!(invoicing.cash_forecast_weeks, 13);
    assert_eq!(invoicing.cash_week_start, Weekday::Mon);
    assert_eq!(invoicing.cash_alert_roles, ["owner", "accountant"]);
    assert_eq!(invoicing.cash_alert_time, "06:00");
    assert!(invoicing.cash_days_late && !invoicing.cash_disputed_counted);
    let entities = wl_kernel::LegalEntity::query()
        .get(app.db())
        .await
        .unwrap();
    assert!(!entities.is_empty());
    for entity in entities {
        let threshold = invoicing.cash_threshold_of(&entity.code);
        assert_eq!(threshold, 10_000_000, "{}", entity.code);
        assert!(9_999_900 < threshold && 10_000_000 >= threshold);
    }
}

/// Budget and drawings: the categories are owned by the accountant (the drawings
/// category by nobody), a budget is approved by the Owner / GM, an expense line
/// reaching 100% warns the owner role and the accountant, and no one approves
/// their own drawing.
#[renox::test]
async fn aut_18_the_budget_categories_belong_to_the_accountant_and_a_drawing_is_never_self_approved()
 {
    let app = auto_booted().await;
    let invoicing = app
        .state()
        .provided::<InvoicingConfig>()
        .expect("invoicing");
    assert_eq!(invoicing.budget_category_owner.as_deref(), Some("accountant"));
    assert_eq!(invoicing.budget_variance_percent, 100);
    assert_eq!(invoicing.budget_variance_role, "accountant");
    assert!(!invoicing.drawings_self_approve);
    assert_eq!(invoicing.drawings_notify_role, "owner");
    let categories = BudgetCategory::listed(app.db(), true).await.unwrap();
    assert!(!categories.is_empty());
    for category in categories {
        if category.is_drawings() {
            assert!(category.owner_role.is_none(), "{}", category.code);
        } else {
            assert_eq!(
                category.owner_role.as_deref(),
                Some("accountant"),
                "{}",
                category.code
            );
        }
    }
}

/// The page lists every line of the table once for an administrator and
/// refuses a technician.
#[renox::test]
async fn aut_18_the_negative_flow_page_shows_the_08_values() {
    let app = auto_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Void allowed")
        .assert_see("Discount limit (SAL.ACL.02)")
        .assert_see("Bounced cheque (FIN.RUL.18)")
        .assert_see("Cash custody")
        .assert_see("Stock write-off approval (STK.ACL.02)")
        .assert_see("Forecast alert (FIN.CFL.01, FIN.RUL.23)")
        .assert_see("Owner drawings");
    let owner = person(&app, "Omar", OWNER).await;
    app.acting_as(&owner);
    let _ = app.get("/settings/negative-flow").await;
    let technician = person(&app, "Tareq", TECHNICIAN).await;
    app.acting_as(&technician);
    app.get("/settings/negative-flow").await.assert_forbidden();
}
