//! BL-LOG-26 (#324), 03 §3.3, §4.1, §4.4 step 5, §7.1, §7.2, §9, `X-01`,
//! `X-02`, `X-05`, `X-11`, `X-12`, `X-18`, `03-N3-a`, `03-N9-a`, `L-20`: the 03
//! deployment's values for the shared delivery, cash, receivables and access
//! mechanisms are in force in a fresh 03 deployment (`wl:install` and the
//! seeds), read from the real modules, not from constants. A test names the
//! Done-when point it covers; the mechanisms themselves (a refused quantity of
//! a multi-parcel shipment, a COD difference, a reversal) are run end to end
//! by the tests of `apps/logistics` and of the modules.

mod support;

use host::roles::{
    ACCOUNTANT, ADMIN, CASHIER, DISPATCHER, DRIVER, MEMBER, OPERATIONS_MANAGER, RECEIVABLES,
    SHIPPING_CLIENT,
};
use logistics::defaults;
use logistics::portal::{self, Access};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_delivery::reasons::FailureReason;
use wl_invoicing::Invoice;
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_kernel::LegalEntity;
use wl_kernel::approvals::{self, ApprovalPolicy};
use wl_kernel::delivery::CodAbsorber;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};

use support::{company, entity, issued_invoice, logistics_booted, messages_to, person};

fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

/// The first day of week `n` (from 1) of the forecast.
fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

fn holds(role: &str, permission: &str) -> bool {
    host::roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .is_some_and(|(_, grants)| grants.contains(&permission))
}

/// AC1: the failure reason list has the five reasons and the abort list
/// *driver unreachable*; a single-parcel shipment has no partial acceptance.
#[renox::test]
async fn the_failure_and_abort_reasons_and_partial_acceptance_are_the_03_values() {
    let app = logistics_booted().await;
    let reasons = FailureReason::active_ones(app.db()).await.unwrap();
    for code in defaults::FAILURE_REASONS {
        assert!(reasons.iter().any(|r| r.code == code), "{code}");
    }
    let count = |code: &str| reasons.iter().find(|r| r.code == code).map(|r| r.counts);
    assert_eq!(count("run_aborted"), Some(false));
    assert_eq!(count("cancelled_by_client"), Some(false));
    assert_eq!(count("wrong_address"), Some(true));

    let settings = wl_delivery::settings::settings(app.state());
    assert!(
        settings
            .abort_reasons()
            .iter()
            .any(|r| r == "driver_unreachable")
    );
    for reason in ["breakdown", "accident", "driver_unwell", "weather", "other"] {
        assert!(
            settings.abort_reasons().iter().any(|r| r == reason),
            "{reason}"
        );
    }
    assert!(
        !settings.partial_acceptance(logistics::SUBJECT_TYPE),
        "a shipment is one consignment: single-parcel offers no refused quantity"
    );
    assert!(settings.hand_over_counts_cash());
    assert!(settings.replace_cash_escalation());
    assert_eq!(settings.untraced_days(), 7);
}

/// AC2: AED 0.01 short needs an operations manager's approval; the approver
/// sees *client*, *business* and *driver* (never the customer); after 10
/// minutes without an answer only the failed attempt is left.
#[renox::test]
async fn a_cod_difference_needs_the_operations_manager_who_chooses_who_absorbs_it() {
    let app = logistics_booted().await;
    let settings = wl_delivery::settings::settings(app.state());
    assert_eq!(settings.cod_tolerance(), 0);
    assert!(settings.cod_approval());
    assert_eq!(settings.cod_approval_minutes(), 10);
    assert_eq!(
        settings.cod_absorbers(),
        [
            CodAbsorber::Client,
            CodAbsorber::Business,
            CodAbsorber::Driver
        ]
    );
    assert!(!settings.offers_absorber(CodAbsorber::Customer));
    // The approver is a holder of the permission: the operations manager and
    // the company admin; the driver cannot approve.
    for role in [OPERATIONS_MANAGER, ADMIN] {
        assert!(
            holds(role, wl_delivery::permissions::APPROVE_COD_DIFFERENCE),
            "{role}"
        );
    }
    for role in [DRIVER, DISPATCHER, CASHIER, ACCOUNTANT] {
        assert!(
            !holds(role, wl_delivery::permissions::APPROVE_COD_DIFFERENCE),
            "{role}"
        );
    }
}

/// The approval policy of `action`: the roles of its first step.
async fn approvers(app: &TestApp, action: &str) -> ApprovalPolicy {
    ApprovalPolicy::for_action(app.db(), action, &renox::serde_json::json!({}))
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the policy {action}"))
}

/// AC3: a dispatcher's reversal request (and a manual lost marking) needs an
/// operations manager's approval; the company admin may approve; the dispatcher
/// who asks and the run's driver cannot approve, and the driver cannot ask.
#[renox::test]
async fn a_reversal_and_a_manual_lost_marking_are_approved_by_the_operations_manager() {
    let app = logistics_booted().await;
    for action in [wl_delivery::reverse::ACTION, wl_delivery::lost::ACTION] {
        let policy = approvers(&app, action).await;
        assert!(policy.sod, "the requester never approves: {action}");
        let step = &policy.steps.0[0];
        assert!(step.allows(OPERATIONS_MANAGER), "{action}");
        assert!(step.allows(ADMIN), "{action}");
        assert!(!step.allows(DISPATCHER), "{action}");
        assert!(!step.allows(DRIVER), "{action}");
    }
    assert!(
        holds(DISPATCHER, wl_delivery::permissions::REVERSE),
        "the dispatcher asks"
    );
    assert!(
        !holds(DRIVER, wl_delivery::permissions::REVERSE),
        "the driver cannot ask"
    );
}

/// #310 AC3, #304: the request to mark a shipment lost from the hub
/// (`logistics.mark_lost`) and the request to dispose of a held one
/// (`logistics.dispose`) are approved by another holder of the approval
/// permission: the operations manager or, where the depot has no other
/// manager, the real company admin role; never the dispatcher who asks.
#[renox::test]
async fn the_hub_loss_and_the_disposal_are_approved_by_the_manager_or_the_real_admin() {
    let app = logistics_booted().await;
    for (action, permission) in [
        (
            logistics::trace::ACTION,
            logistics::permissions::SHIPMENTS_APPROVE_LOSS,
        ),
        (
            logistics::disposal::ACTION,
            logistics::permissions::SHIPMENTS_APPROVE_DISPOSAL,
        ),
    ] {
        let policy = approvers(&app, action).await;
        assert!(policy.sod, "the requester never approves: {action}");
        let step = &policy.steps.0[0];
        assert_eq!(step.permission.as_deref(), Some(permission), "{action}");
        assert!(holds(OPERATIONS_MANAGER, permission), "{action}");
        assert!(holds(ADMIN, permission), "the company admin: {action}");
        assert!(!holds(DISPATCHER, permission), "{action}");
        assert!(!holds(DRIVER, permission), "{action}");
    }
}

/// AC4 at the boundary: AED 50.00 is written off by the cashier, AED 50.01
/// needs the approval of another holder of `cash.write_off`.
#[renox::test]
async fn a_shortage_of_aed_50_00_needs_no_approval_and_50_01_does() {
    let app = logistics_booted().await;
    let at = |amount: i64| {
        let db = app.db();
        async move {
            ApprovalPolicy::for_action(
                db,
                wl_payments::cash_settle::ACTION,
                &renox::serde_json::json!({ "amount": amount }),
            )
            .await
            .unwrap()
        }
    };
    assert!(at(5_000).await.is_none(), "AED 50.00 is within the limit");
    assert!(at(5_001).await.is_some(), "AED 50.01 needs an approval");
}

/// AC4: a AED 40 shortage can be written off by the cashier, AED 60 needs an
/// approval of another holder of `cash.write_off`; salary deduction is not
/// offered.
#[renox::test]
async fn a_shortage_of_aed_40_is_written_off_by_the_cashier_and_aed_60_needs_approval() {
    let app = logistics_booted().await;
    assert_eq!(wl_payments::cash_settle::limit(app.state()), 5_000);
    assert!(holds(CASHIER, wl_payments::permissions::CASH_WRITE_OFF));
    assert!(holds(CASHIER, wl_payments::permissions::CASH_COUNT));
    let within = ApprovalPolicy::for_action(
        app.db(),
        wl_payments::cash_settle::ACTION,
        &renox::serde_json::json!({ "amount": 4_000 }),
    )
    .await
    .unwrap();
    assert!(within.is_none(), "AED 40 is within the limit");
    let above = ApprovalPolicy::for_action(
        app.db(),
        wl_payments::cash_settle::ACTION,
        &renox::serde_json::json!({ "amount": 6_000 }),
    )
    .await
    .unwrap()
    .expect("AED 60 needs an approval");
    assert!(above.sod, "another user than the requester approves");
    let payments = host::payments_for("logistics");
    let config = payments.config();
    assert!(!config.cash_salary_deduction, "L-20: off, a setting");
    assert_eq!(config.cash_manager_role, OPERATIONS_MANAGER);
    // Turning it on is the company's decision, in the setting, not in code.
    assert!(
        host::payments_for("logistics")
            .cash_salary_deduction(true)
            .config()
            .cash_salary_deduction
    );
}

/// AC5: every invoice write-off waits for the operations manager or the
/// company admin, never the requester, and the client is told; a returned
/// cheque adds no bounce fee; a client user can dispute an invoice in the
/// portal, escalated after 14 days.
#[renox::test]
async fn every_invoice_write_off_needs_the_operations_manager_or_the_admin() {
    let app = logistics_booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let manager = person(&app, "Olivia", OPERATIONS_MANAGER).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let dispatcher = person(&app, "Dina", DISPATCHER).await;
    let client = company(&app, "Souq Express Trading LLC").await;
    let due = today() - Days::new(30);
    let invoice = issued_invoice(&app, &clerk, client, 100, VatCategory::Standard, due).await;

    app.acting_as(&accountant);
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
    assert_eq!(pending.len(), 1, "AED 0.01 is above the limit of AED 0");
    let waiting = |user: &User| {
        let (user, db) = (user.clone(), app.db().clone());
        async move {
            approvals::awaiting(&db, &user)
                .await
                .unwrap()
                .iter()
                .any(|r| r.action == "invoicing.write_off")
        }
    };
    assert!(waiting(&manager).await);
    assert!(waiting(&admin).await);
    assert!(!waiting(&accountant).await, "the requester cannot approve");
    assert!(!waiting(&dispatcher).await);
    let own = approvals::approve(app.state(), pending[0].id, &accountant, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &manager, "agreed")
        .await
        .unwrap();
    let after = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    // The client's write-off notice is sent by the queued job and dated.
    assert_eq!(after.write_off_notice_on, None, "not before the job");
    app.run_all_jobs().await;
    let noticed = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(
        noticed.write_off_notice_on,
        Some(today()),
        "the client received the write-off notice"
    );

    let invoicing = host::invoicing_for("logistics", &["NEC".to_owned()]);
    let config = invoicing.config();
    assert!(
        config.write_off_notice,
        "the client receives the write-off notice"
    );
    assert!(config.bad_debt_relief, "FIN.RUL.14 proposes the relief");
    assert!(
        config.portal_disputes,
        "a client user disputes in the portal"
    );
    assert_eq!(config.dispute_escalation_days, 14);
    assert!(
        host::payments_for("logistics")
            .config()
            .bounce_fee
            .is_none(),
        "a returned cheque adds no fee"
    );
}

/// AC5: a client user disputes an invoice of its own company in the portal
/// (not another company's), and the dispute is escalated 14 days after it was raised
/// (the owner of the invoice) and again after 28 (the accountant).
#[renox::test]
async fn a_client_disputes_in_the_portal_and_the_dispute_escalates_after_14_days() {
    use wl_invoicing::{dispute, escalation};
    use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId};
    let app = logistics_booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let client = company(&app, "Souq Express Trading LLC").await;
    let other = company(&app, "Other Trading LLC").await;
    let invoice = issued_invoice(
        &app,
        &clerk,
        client,
        100_000,
        VatCategory::Standard,
        today() + Days::new(30),
    )
    .await;

    let by_other = dispute::raise(
        app.state(),
        DisputeRequest::new(InvoiceId(invoice.id), "not ours", DisputeBy::Party { id: other }),
    )
    .await;
    assert!(by_other.is_err(), "another company cannot dispute it");
    dispute::raise(
        app.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "three shipments missing",
            DisputeBy::Party { id: client },
        )
        .amount(10_000),
    )
    .await
    .expect("the client disputes its own invoice");
    assert!(
        Invoice::find_or_404(app.db(), invoice.id)
            .await
            .unwrap()
            .disputed
    );

    let before = escalation::run(app.state(), today() + Days::new(13))
        .await
        .unwrap();
    assert_eq!(before, 0, "nothing before day 14");
    let sent = escalation::run(app.state(), today() + Days::new(14))
        .await
        .unwrap();
    assert!(sent >= 1, "escalated on day 14");
    assert!(
        !messages_to(&app, escalation::ESCALATED, &clerk)
            .await
            .is_empty(),
        "the invoice owner is told first"
    );
    assert!(
        messages_to(&app, escalation::ESCALATED, &accountant)
            .await
            .is_empty(),
        "the escalation role follows at the next step"
    );
    escalation::run(app.state(), today() + Days::new(28))
        .await
        .unwrap();
    assert!(
        !messages_to(&app, escalation::ESCALATED, &accountant)
            .await
            .is_empty(),
        "the accountant is told after 28 days"
    );
}

/// AC6: a blocked client's remittance exception is approved by the company
/// admin; a client whose contract ended 2026-11-01 has read-only portal access
/// until 2027-01-30 and none from 2027-01-31.
#[renox::test]
async fn the_exception_approver_is_the_admin_and_the_portal_closes_after_90_days() {
    let app = logistics_booted().await;
    assert_eq!(host::parties_for("logistics").approver(), "admin");

    let manager = person(&app, "Olivia", OPERATIONS_MANAGER).await;
    let client = company(&app, "Souq Express Trading LLC").await;
    app.acting_as(&manager);
    app.post(
        &format!("/clients/{}/contract-end", client.0),
        &[("date", "2026-11-01")],
    )
    .await
    .assert_status(303);
    let day = |text: &str| -> NaiveDate { text.parse().unwrap() };
    for (on, access) in [
        ("2026-11-01", Access::Open),
        ("2026-11-02", Access::ReadOnly),
        ("2027-01-30", Access::ReadOnly),
        ("2027-01-31", Access::Closed),
    ] {
        assert_eq!(
            portal::access(app.db(), client, day(on)).await.unwrap(),
            access,
            "{on}"
        );
    }
    // A user who may not manage shipments cannot record it.
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    app.post(
        &format!("/clients/{}/contract-end", client.0),
        &[("date", "2026-12-01")],
    )
    .await
    .assert_forbidden();
    // Taking the end back opens the portal again.
    app.acting_as(&manager);
    app.post(
        &format!("/clients/{}/contract-end", client.0),
        &[("date", "")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        portal::access(app.db(), client, day("2030-01-01"))
            .await
            .unwrap(),
        Access::Open
    );
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

/// A fresh 03 deployment with the company admin and the accountant.
async fn scene() -> (TestApp, LegalEntity, User, User) {
    let app = logistics_booted().await;
    let admin = person(&app, "Alia", ADMIN).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    SupplierBill::query().delete(app.db()).await.unwrap();
    let nec = entity(&app, "NEC").await;
    (app, nec, admin, accountant)
}

async fn told(app: &TestApp, user: &User) -> usize {
    messages_to(app, BELOW_THRESHOLD, user).await.len()
}

/// AC7: a projected balance of AED 99,999 in week 5 alerts the company admin
/// and the accountant once; AED 100,000 in every week alerts no one.
#[renox::test]
async fn ninety_nine_thousand_nine_hundred_ninety_nine_in_week_five_alerts_once() {
    let (app, nec, admin, accountant) = scene().await;
    // AED 200,000 in the bank; a bill of AED 100,001 falls due in week 5.
    bank(&app, &nec, 20_000_000).await;
    bill_due(&app, &nec, "SB-W5", week(5) + Days::new(2), 10_000_100).await;

    let sent = cash_alert::run(app.state(), today()).await.unwrap();
    assert_eq!(sent, 1);
    assert_eq!(told(&app, &admin).await, 1);
    assert_eq!(told(&app, &accountant).await, 1);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &admin).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 99,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 100,000.00");
    // The next daily run says nothing.
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &admin).await, 1);
}

#[renox::test]
async fn one_hundred_thousand_in_every_week_alerts_no_one() {
    let (app, nec, admin, accountant) = scene().await;
    bank(&app, &nec, 20_000_000).await;
    bill_due(&app, &nec, "SB-W5", week(5) + Days::new(2), 10_000_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &admin).await, 0);
    assert_eq!(told(&app, &accountant).await, 0);
}

/// AC7: the forecast has 13 weekly buckets from Monday and the check runs at
/// 06:00 Asia/Dubai.
#[renox::test]
async fn the_forecast_has_thirteen_weeks_from_monday_and_runs_at_six() {
    let (app, nec, _admin, _accountant) = scene().await;
    bank(&app, &nec, 20_000_000).await;
    let module = host::invoicing_for("logistics", &["NEC".to_owned()]);
    let config = module.config().clone();
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&nec), today())
            .await
            .unwrap();
    let forecast: Forecast = cash_forecast::assemble(&inputs);
    let weeks = &forecast.weeks;
    assert_eq!(weeks.len(), 13);
    assert_eq!(weeks[0].start, this_monday());
    assert!(weeks.iter().all(|w| w.start.weekday() == Weekday::Mon));

    use renox::Module;
    let mut registry = renox::Registry::default();
    module.register(&mut registry);
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

/// The COD owed to clients is a payment of the forecast: an approved
/// remittance of AED 20,000 takes AED 110,000 to AED 90,000, below the
/// threshold; a draft counts at the next Monday run.
#[renox::test]
async fn a_cod_remittance_owed_to_a_client_counts_as_a_payment() {
    let (app, nec, admin, _accountant) = scene().await;
    bank(&app, &nec, 11_000_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);

    let client = company(&app, "Souq Express Trading LLC").await;
    let mut remittance = logistics::Remittance {
        number: "REM-0001".to_owned(),
        client_party_id: client.0,
        period_end: renox::db::now(),
        net: 2_000_000,
        status: logistics::RemittanceStatus::Approved,
        ..logistics::Remittance::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    remittance
        .insert_in_state(&mut tx, logistics::RemittanceStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let sent = cash_alert::run(app.state(), today()).await.unwrap();
    assert_eq!(
        sent, 1,
        "AED 110,000 less the AED 20,000 owed to the client"
    );
    let notice = &messages_to(&app, BELOW_THRESHOLD, &admin).await[0];
    assert_eq!(notice.data.0["balance"], "AED 90,000.00");
    assert!(
        notice.data.0["payments"]
            .as_str()
            .unwrap_or_default()
            .contains("REM-0001"),
        "{}",
        notice.data.0["payments"]
    );
}

/// AC8: the budget of a legal entity prepared by the accountant is approved by
/// the company admin; a drawing is never approved by its requester;
/// `FIN.RUL.24` warns the category's owner role and the accountant at 100% of
/// a month's line. The values are the 03 ones in the module's configuration.
#[renox::test]
async fn the_budget_is_approved_by_the_admin_and_the_drawings_by_someone_else() {
    let app = logistics_booted().await;
    let module = host::invoicing_for("logistics", &["NEC".to_owned()]);
    let config = module.config();
    assert!(!config.drawings_self_approve);
    assert_eq!(config.drawings_notify_role, "admin");
    assert_eq!(config.budget_variance_percent, 100);
    assert_eq!(config.budget_variance_role, "accountant");
    assert_eq!(config.cash_alert_roles, ["admin", "accountant"]);
    assert_eq!(config.cash_threshold_of("NEC"), 10_000_000);
    // The accountant prepares the budget; only the company admin approves it.
    assert!(holds(ACCOUNTANT, wl_invoicing::permissions::BUDGETS_MANAGE));
    assert!(!holds(
        ACCOUNTANT,
        wl_invoicing::permissions::BUDGETS_APPROVE
    ));
    assert!(holds(ADMIN, wl_invoicing::permissions::BUDGETS_APPROVE));
    // The budget categories have the accountant as owner role.
    let categories = wl_invoicing::BudgetCategory::listed(app.db(), true)
        .await
        .unwrap();
    for category in categories.iter().filter(|c| !c.is_drawings()) {
        assert_eq!(
            category.owner_role.as_deref(),
            Some("accountant"),
            "{}",
            category.name_en
        );
    }
}

/// AC8 (behaviour): the accountant prepares the 2026-27 budget of NEC and only
/// the company admin approves it; a supplier bill coded to *marketing* that
/// brings the month to 100% of its line warns the accountant (owner of the
/// category and variance role) once; a bill coded to owner drawings waits for
/// the admin, never for the user who entered it, and stays out of the operating
/// cost.
#[renox::test]
async fn the_budget_is_approved_by_the_admin_and_overspending_warns_the_accountant_once() {
    use wl_invoicing::bill_machine;
    use wl_invoicing::bill_submit;
    use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
    use wl_invoicing::credit_notes::By;
    use wl_invoicing::supplier_bill::{self, BillDraft};
    use wl_invoicing::{
        BillLineDraft, BudgetCategory, BudgetKind, CompanyBudget, DrawingApproval, DrawingStatus,
        StartChoice,
    };
    let app = logistics_booted().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let admin = person(&app, "Ada", ADMIN).await;
    let nec = entity(&app, "NEC").await;
    let supplier = company(&app, "Ad Agency LLC").await;
    let this_month = today().with_day(1).unwrap();

    // The accountant prepares and submits; the request waits for the admin.
    let draft = budget_lines::create(
        app.state(),
        NewBudget {
            legal_entity_id: nec.id,
            fiscal_year: i64::from(this_month.year()),
            name: String::new(),
            start: StartChoice::Empty,
            revision_reason: None,
        },
        Some(accountant.id),
    )
    .await
    .unwrap();
    let marketing = BudgetCategory::by_code(app.db(), "marketing")
        .await
        .unwrap()
        .expect("the marketing category");
    let lines: Vec<LineInput> = draft
        .months()
        .into_iter()
        .map(|month| LineInput {
            category_id: marketing.id,
            month,
            amount: 1_000_000,
            note: None,
        })
        .collect();
    budget_lines::set_lines(app.db(), draft.id, &lines).await.unwrap();
    app.acting_as(&accountant);
    app.htmx()
        .post(&format!("/budgets/{}/submit", draft.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", draft.id));
    let pending = approvals::pending_for(app.db(), &draft.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert!(
        approvals::approve(app.state(), pending[0].id, &accountant, "")
            .await
            .is_err(),
        "the preparer cannot approve the budget"
    );
    approvals::approve(app.state(), pending[0].id, &admin, "")
        .await
        .unwrap()
        .expect("the admin approves");
    let approved = CompanyBudget::find(app.db(), draft.id).await.unwrap().unwrap();
    assert_eq!(approved.status, wl_invoicing::BudgetStatus::Approved);

    // Two bills take March of the line (AED 10,000) to 100%: one warning.
    let bill = |number: &'static str, price: i64, code: String| {
        let (state, db) = (app.state().clone(), app.db().clone());
        let (nec_id, by) = (nec.id, accountant.id);
        async move {
            let mut draft = BillDraft::new(supplier, number, today());
            draft.legal_entity_id = Some(nec_id);
            draft = draft.line(
                BillLineDraft::new("Ads", price, VatCategory::Standard).cost_code(&code),
            );
            let bill = supplier_bill::create(&db, &draft, Some(by)).await.unwrap();
            bill_submit::submit(&state, By::System(Some(by)), bill.id)
                .await
                .unwrap();
            bill
        }
    };
    let first = bill("SB-1", 600_000, "marketing".to_owned()).await;
    bill_machine::approve(app.state(), By::System(Some(accountant.id)), first.id, "")
        .await
        .unwrap();
    assert!(
        messages_to(&app, "invoicing.budget_variance", &accountant)
            .await
            .is_empty(),
        "60% of the line: no warning"
    );
    let second = bill("SB-2", 450_000, "marketing".to_owned()).await;
    bill_machine::approve(app.state(), By::System(Some(accountant.id)), second.id, "")
        .await
        .unwrap();
    let third = bill("SB-3", 100_000, "marketing".to_owned()).await;
    bill_machine::approve(app.state(), By::System(Some(accountant.id)), third.id, "")
        .await
        .unwrap();
    assert_eq!(
        messages_to(&app, "invoicing.budget_variance", &accountant)
            .await
            .len(),
        1,
        "the accountant is warned once, however many bills follow"
    );

    // A drawing waits for the admin and is never approved by its requester.
    let drawings = BudgetCategory::listed(app.db(), true)
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.kind == BudgetKind::OwnerDrawings)
        .expect("the drawings category");
    let drawn = bill("SB-4", 200_000, drawings.code.clone()).await;
    let requested = DrawingApproval::for_source(app.db(), &drawn.subject())
        .await
        .unwrap();
    let request = requested
        .iter()
        .find(|d| d.status == DrawingStatus::Requested)
        .and_then(|d| d.approval_request_id)
        .expect("the drawing waits for an approval");
    assert!(
        approvals::approve(app.state(), request, &accountant, "")
            .await
            .is_err(),
        "the requester cannot approve its own drawing"
    );
    approvals::approve(app.state(), request, &admin, "")
        .await
        .unwrap()
        .expect("the admin approves the drawing");
}

/// The money page shows the 03 rows, each with its value and who set it; a
/// member and a client user do not see it.
#[renox::test]
async fn an_admin_sees_the_03_money_settings_and_a_member_does_not() {
    let app = logistics_booted().await;
    let admin = person(&app, "Ada", ADMIN).await;
    app.acting_as(&admin);
    app.get("/settings/negative-flow")
        .await
        .assert_ok()
        .assert_see("Money settings")
        .assert_see("Cash shortage write-off limit")
        .assert_see("AED 50.00")
        .assert_see("L-20")
        .assert_see("Portal after contract end")
        .assert_see("AED 100,000.00");
    let member = person(&app, "Mia", MEMBER).await;
    app.acting_as(&member);
    app.get("/settings/negative-flow").await.assert_forbidden();
    let client = person(&app, "Noor", SHIPPING_CLIENT).await;
    app.acting_as(&client);
    app.get("/settings/negative-flow").await.assert_forbidden();
}
