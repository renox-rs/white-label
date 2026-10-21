//! BL-MPW-21 (#435): the 09 values for the shared money, attendance, payroll
//! and HR mechanisms, run by the real modules of the host in a fresh manpower
//! agency (`wl:install` and the seeds), not as constants: the write-off and
//! dispute settings, the cheque paths and the missing bounce fee, the
//! overtime and flag review, the provisional payroll at the cut-off, the HR
//! cooling, caps and exits, the settlement alerts, the cash-flow forecast and
//! the budget with owner drawings. `FIN.ACL.15`, `FIN.RUL.15`, `FIN.RUL.18`,
//! `HRM.RUL.02`, `HRM.RUL.09`, `HRM.RUL.13`, `HRM.RUL.15`, `HRM.ACL.05`,
//! `HRM.ACL.06`, `HRM.ACL.09`, `FIN.RUL.23`, `FIN.RUL.24`, `FIN.ACL.21`.

mod manpower_support;
mod support;

use host::roles::{ACCOUNTANT, ADMIN, HR, OWNER, RECEIVABLES, SUPERVISOR};
use manpower_support::{boot_seeded, worker};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, day, entity, issued_invoice, messages_to, person};
use wl_invoicing::budget_lines::{self, LineInput, NewBudget};
use wl_invoicing::budget_report::{self, ReportQuery};
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_invoicing::{
    BudgetCategory, BudgetStatus, CompanyBudget, DrawingApproval, DrawingStatus, EscalationLog,
    Invoice, StartChoice, cash_forecast_load, escalation,
};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::cheques::{ChequeStatus, RegisterCheque, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DisputeBy, DisputeRequest, InvoiceId, InvoiceStatus, VatCategory};
use wl_kernel::{LegalEntity, Subject};

// ---- Invoices ---------------------------------------------------------------

/// AC1 (Invoices): a write-off of AED 500 on a 09 invoice needs the MD's
/// approval (limit AED 0) and never the requester's; the client approver can
/// flag an invoice `disputed` in the portal; an open dispute escalates after 14
/// days.
#[renox::test]
async fn a_write_off_of_aed_500_waits_for_the_md_and_a_dispute_escalates_after_14_days() {
    let app = boot_seeded().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let md = person(&app, "Mona", ADMIN).await;
    let client = company(&app, "Gulf Tower LLC").await;
    let invoice = issued_invoice(&app, &clerk, client, 50_000, VatCategory::Exempt, day(-30)).await;

    // The write-off waits for the MD.
    app.acting_as(&clerk);
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
    assert_eq!(pending.len(), 1, "AED 500 is above the limit of AED 0");
    let own = approvals::approve(app.state(), pending[0].id, &clerk, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), pending[0].id, &md, "agreed")
        .await
        .unwrap();
    assert_eq!(
        Invoice::find_or_404(app.db(), invoice.id)
            .await
            .unwrap()
            .status,
        InvoiceStatus::WrittenOff
    );

    // The client approver flags another invoice `disputed` in the portal.
    let second = issued_invoice(&app, &clerk, client, 100_000, VatCategory::Exempt, day(-30)).await;
    let id = dispute::raise(
        app.state(),
        DisputeRequest::new(
            InvoiceId(second.id),
            "hours not worked",
            DisputeBy::Party { id: client },
        )
        .amount(20_000),
    )
    .await
    .expect("the client may dispute from the portal");
    assert!(
        Invoice::find_or_404(app.db(), second.id)
            .await
            .unwrap()
            .disputed
    );

    // Open 14 days: escalated (`FIN.RUL.15`); the day before nothing.
    let mut row = Dispute::find_or_404(app.db(), id).await.unwrap();
    row.raised_at = (today() - Days::new(14))
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc();
    row.save(app.db()).await.unwrap();
    escalation::run(app.state(), today() - Days::new(1))
        .await
        .unwrap();
    assert!(
        EscalationLog::told(app.db(), id, 1)
            .await
            .unwrap()
            .is_empty()
    );
    escalation::run(app.state(), today()).await.unwrap();
    assert!(
        EscalationLog::told(app.db(), id, 1)
            .await
            .unwrap()
            .contains(&clerk.id),
        "the invoice owner is reminded at 14 days"
    );
}

// ---- Cheques ----------------------------------------------------------------

/// AC2 (Cheques): a bounced client cheque offers re-present, the legal case and
/// write-off, and creates no bounce fee charge.
#[renox::test]
async fn a_bounced_client_cheque_offers_the_three_paths_and_adds_no_fee() {
    use wl_payments::Cheque;
    use wl_payments::cheque::ChequeReturn;

    let app = boot_seeded().await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let client = company(&app, "Gulf Tower LLC").await;
    let entity = entity(&app, "GMS").await;
    let summary = cheques(app.state())
        .register(
            app.state(),
            RegisterCheque::new(client, entity.id, "ENBD", "2001", day(0), 1_000_000),
            Some(accountant.id),
        )
        .await
        .expect("the cheque is registered");
    let cheque = Cheque::find_or_404(app.db(), summary.id.0).await.unwrap();
    app.acting_as(&accountant);
    app.htmx()
        .post(&format!("/cheques/{}/deposit", cheque.id), &[])
        .await
        .assert_ok();
    app.htmx()
        .post(
            &format!("/cheques/{}/bounce", cheque.id),
            &[
                ("reason", "insufficient_funds"),
                ("certificate", "CERT-77"),
                ("honoured", ""),
            ],
        )
        .await
        .assert_ok();
    let bounced = Cheque::find_or_404(app.db(), cheque.id).await.unwrap();
    assert_eq!(bounced.status, ChequeStatus::Bounced);
    assert_eq!(bounced.bounce_fee_total, 0, "no bounce fee by default");
    let returns = ChequeReturn::of_cheque(app.db(), cheque.id).await.unwrap();
    assert_eq!(returns[0].fee_amount, 0);
    assert_eq!(returns[0].fee_subject_id, None, "no fee charge was created");

    // The three paths after `bounced`.
    let machine = Cheque::machine();
    for path in ["represent", "refer_to_legal", "write_off"] {
        let transition = machine
            .find(path)
            .unwrap_or_else(|_| panic!("the transition {path}"));
        assert!(
            transition.from_states().iter().any(|s| s == "bounced"),
            "{path} is offered from bounced"
        );
    }
}

// ---- Attendance -------------------------------------------------------------

/// AC3 (Attendance): 3 overtime hours on a day are flagged `overtime_cap`; the
/// supervisor who marked the record cannot accept the flag, another reviewer
/// can with a reason; the escalation goes at day 6.
#[renox::test]
async fn three_overtime_hours_are_flagged_and_the_marking_supervisor_cannot_accept_them() {
    use wl_attendance::service::{self, NewRecord};
    use wl_attendance::{AttendanceRecord, AttendanceStatus, WorkLocation};

    let app = boot_seeded().await;
    let boss = person(&app, "Bashir", SUPERVISOR).await;
    // The supervisor also holds the review permission (HR): the rule still
    // keeps them from accepting their own mark.
    boss.assign_role(app.db(), HR).await.unwrap();
    let reviewer = person(&app, "Hana", HR).await;
    let w = worker(&app, "Wasim Khan").await;
    let site = WorkLocation::create(app.db(), WorkLocation::named("Tower A"))
        .await
        .unwrap();
    let calendar = wl_kernel::BusinessCalendar::default();
    let at = |hour: u32| calendar.from_local(day(0).and_hms_opt(hour, 0, 0).unwrap());
    // 07:00 to 18:00 is 11 hours: 8 normal and 3 overtime, above the 2.
    let record = service::record(
        app.state(),
        NewRecord::supervisor(w, site.id, at(7), Some(at(18)), boss.id),
    )
    .await
    .unwrap();
    let flagged = AttendanceRecord::find_or_404(app.db(), record.id)
        .await
        .unwrap();
    assert_eq!(flagged.status, AttendanceStatus::Flagged);
    let flags = wl_attendance::AttendanceFlag::query()
        .where_(wl_attendance::AttendanceFlag::RECORD_ID.eq(record.id))
        .get(app.db())
        .await
        .unwrap();
    assert!(
        flags
            .iter()
            .any(|f| f.flag_type == wl_attendance::FlagType::OvertimeCap)
    );

    app.acting_as(&boss);
    let own = app
        .htmx()
        .post(
            &format!("/attendance/records/{}/accept", record.id),
            &[("reason", "I marked it myself")],
        )
        .await;
    assert!(
        [403, 422].contains(&own.status.as_u16()),
        "the marking supervisor cannot accept: {}",
        own.status
    );
    app.acting_as(&reviewer);
    app.htmx()
        .post(
            &format!("/attendance/records/{}/accept", record.id),
            &[("reason", "approved overtime for the audit")],
        )
        .await
        .assert_ok();
    assert_eq!(
        AttendanceRecord::find_or_404(app.db(), record.id)
            .await
            .unwrap()
            .status,
        AttendanceStatus::Accepted
    );
    let config = wl_attendance::config::config_of(app.state());
    assert_eq!(config.escalation_days, 6, "the escalation goes at day 6");
}

// ---- Payroll ----------------------------------------------------------------

/// AC4 (Payroll): the payroll cut-off is the 25th; at the cut-off a deployment
/// with an unapproved timesheet is paid on provisional attendance; switching the
/// payroll basis of an external approver to `wait` is refused.
#[renox::test]
async fn at_the_cut_off_an_unapproved_timesheet_is_paid_on_provisional_attendance() {
    use wl_attendance::config::PayrollBasis;
    use wl_attendance::timesheet::ApproverKind;
    use wl_attendance::{AttendanceRecord, Timesheet, TimesheetStatus};
    use wl_kernel::timesheets::{PayrollPeriod, timesheets};

    let app = boot_seeded().await;
    assert_eq!(wl_hr::config::config_of(app.state()).cut_off_day, 25);

    let w = worker(&app, "Wasim Khan").await;
    let site = wl_attendance::WorkLocation::create(
        app.db(),
        wl_attendance::WorkLocation::named("Tower A"),
    )
    .await
    .unwrap();
    let calendar = wl_kernel::BusinessCalendar::default();
    let nov = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    // Four accepted days of 8 hours before the cut-off, one after it.
    for d in [2, 3, 4, 5, 26] {
        let mut record = AttendanceRecord {
            employee_id: w.0,
            location_id: site.id,
            date: nov(d),
            check_in: calendar.from_local(nov(d).and_hms_opt(7, 0, 0).unwrap()),
            check_out: Some(calendar.from_local(nov(d).and_hms_opt(15, 0, 0).unwrap())),
            status: wl_attendance::AttendanceStatus::Accepted,
            method: wl_attendance::record::AttendanceMethod::Supervisor,
            ..AttendanceRecord::default()
        };
        let mut tx = app.db().begin().await.unwrap();
        record
            .insert_in_state(
                &mut tx,
                wl_attendance::AttendanceStatus::Accepted,
                "fixture",
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let context = Subject::new("manpower.deployment", 9);
    let mut sheet = Timesheet::regular(w, nov(1), nov(30)).in_context(&context);
    sheet.approver_kind = ApproverKind::External;
    sheet.approver_role = Some("client_approver".into());
    sheet.status = TimesheetStatus::Submitted;
    let mut tx = app.db().begin().await.unwrap();
    sheet
        .insert_in_state(&mut tx, TimesheetStatus::Submitted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let period = PayrollPeriod::new(nov(1), nov(30), nov(25));
    let found = timesheets(app.state())
        .hours_for_payroll(app.state(), &period)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert!(
        found[0].provisional,
        "paid on attendance, marked provisional"
    );
    assert_eq!(
        found[0].total_minutes(),
        4 * 8 * 60,
        "the day after the cut-off waits"
    );

    let module = host::manpower_seed::attendance(wl_attendance::WlAttendance::new());
    assert!(
        module
            .payroll_basis(ApproverKind::External, PayrollBasis::Wait)
            .is_err(),
        "payroll never waits for the client"
    );
}

// ---- HR ---------------------------------------------------------------------

/// AC5 (HR): the new pay account cools for 5 days (`HRM.ACL.05`), a recovery of
/// 25% of the wage is above the overpayment cap of 20% and the total of 50%
/// stands (`HRM.ACL.06`), and the absconding report is approved by the MD
/// (`HRM.ACL.07`).
#[renox::test]
async fn the_hr_values_are_in_force_cooling_five_days_and_the_deduction_caps() {
    use wl_hr::AdjustmentKind;
    use wl_hr::caps::{CapScope, check};

    let app = boot_seeded().await;
    let config = wl_hr::config::config_of(app.state());
    assert_eq!(config.cooling_days, 5);
    assert_eq!(config.absence_days, 7);
    assert_eq!(config.settlement_alert_days, 10);
    assert_eq!(config.wps_escalation_days, 1);
    // A wage of AED 3,000: a recovery of 25% (AED 750) is above the cap of 20%
    // (AED 600), and 51% together is above the total of 50%.
    let wage = 300_000;
    let problems = check(1, 7, wage, &[(AdjustmentKind::OverpaymentRecovery, 75_000)]);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, CapScope::Overpayment);
    assert_eq!((problems[0].amount, problems[0].cap), (75_000, 60_000));
    assert!(
        check(1, 7, wage, &[(AdjustmentKind::OverpaymentRecovery, 60_000)]).is_empty(),
        "20% is within the cap"
    );
    let total = check(
        1,
        7,
        wage,
        &[
            (AdjustmentKind::OverpaymentRecovery, 60_000),
            (AdjustmentKind::AdvanceRecovery, 93_000),
        ],
    );
    assert_eq!(total.len(), 1);
    assert_eq!(total[0].scope, CapScope::Total);
    // The MD who approves the non-standard exits is a role of the host.
    assert!(host::roles::ROLES.iter().any(|(name, grants)| {
        *name == OWNER && grants.contains(&wl_hr::permissions::EMPLOYEES_EXIT_APPROVE)
    }));
}

/// AC6 (Settlement): given a last working day of 2026-11-30 and the settlement
/// not `paid`, HR and the accountant are alerted on 2026-12-10 and the MD on
/// 2026-12-14.
#[renox::test]
async fn the_settlement_alerts_go_at_ten_and_fourteen_days_after_the_last_working_day() {
    use wl_hr::{FinalSettlement, SettlementStatus, settlement_alerts};

    let app = boot_seeded().await;
    let hr = person(&app, "Hira", HR).await;
    let accountant = person(&app, "Abdul", ACCOUNTANT).await;
    let md = person(&app, "Mazen", ADMIN).await;
    let w = worker(&app, "Ravi Kumar").await;
    let mut settlement = FinalSettlement {
        employee_id: w.0,
        reason: "resignation".into(),
        payee: "employee".into(),
        statement_version: 1,
        status: SettlementStatus::Approved,
        last_working_day: Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()),
        unpaid_salary: 1_200_000,
        net: 1_200_000,
        ..FinalSettlement::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    settlement
        .insert_in_state(&mut tx, SettlementStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let dec = |d: u32| NaiveDate::from_ymd_opt(2026, 12, d).unwrap();
    settlement_alerts::run(app.state(), dec(9)).await.unwrap();
    assert!(messages_to(&app, "hr.settlement_due", &hr).await.is_empty());
    settlement_alerts::run(app.state(), dec(10)).await.unwrap();
    assert_eq!(messages_to(&app, "hr.settlement_due", &hr).await.len(), 1);
    assert_eq!(
        messages_to(&app, "hr.settlement_due", &accountant)
            .await
            .len(),
        1
    );
    assert!(
        messages_to(&app, "hr.settlement_overdue", &md)
            .await
            .is_empty()
    );
    settlement_alerts::run(app.state(), dec(13)).await.unwrap();
    assert!(
        messages_to(&app, "hr.settlement_overdue", &md)
            .await
            .is_empty()
    );
    settlement_alerts::run(app.state(), dec(14)).await.unwrap();
    assert_eq!(
        messages_to(&app, "hr.settlement_overdue", &md).await.len(),
        1
    );
}

// ---- Cash-flow forecast -----------------------------------------------------

/// The Monday of the week that holds today.
fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

/// The first day of week `n` (from 1) of the forecast.
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

/// AC7 (Cash-flow forecast): a projected balance of AED 249,999 in week 5
/// alerts the MD and the accountant once; AED 250,000 in every week alerts no
/// one; the forecast has 13 weekly buckets from Monday.
#[renox::test]
async fn two_hundred_forty_nine_thousand_in_week_five_alerts_the_md_and_the_accountant_once() {
    let app = boot_seeded().await;
    let md = person(&app, "Alia", ADMIN).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    // The demo data has a bill of its own: the checks start from none.
    SupplierBill::query().delete(app.db()).await.unwrap();
    let gms = entity(&app, "GMS").await;
    bank(&app, &gms, 30_000_000).await;
    // AED 300,000 in the bank; a bill of AED 50,001 falls due in week 5.
    bill_due(&app, &gms, "SB-W5", week(5) + Days::new(2), 5_000_100).await;

    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 1);
    assert_eq!(told(&app, &md).await, 1);
    assert_eq!(told(&app, &accountant).await, 1);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &md).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 249,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 250,000.00");
    assert_eq!(
        cash_alert::run(app.state(), today()).await.unwrap(),
        0,
        "once"
    );

    let config = host::invoicing_for("manpower", &["GMS".to_owned()])
        .config()
        .clone();
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&gms), today())
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
}

#[renox::test]
async fn two_hundred_fifty_thousand_in_every_week_alerts_no_one() {
    let app = boot_seeded().await;
    let md = person(&app, "Alia", ADMIN).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    // The demo data has a bill of its own: the checks start from none.
    SupplierBill::query().delete(app.db()).await.unwrap();
    let gms = entity(&app, "GMS").await;
    bank(&app, &gms, 30_000_000).await;
    // AED 50,000 of payments leaves exactly AED 250,000.
    bill_due(&app, &gms, "SB-W5", week(5) + Days::new(2), 5_000_000).await;

    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &md).await, 0);
    assert_eq!(told(&app, &accountant).await, 0);
}

// ---- Budget and owner drawings ----------------------------------------------

const VARIANCE: &str = "invoicing.budget_variance";

async fn draft_budget(app: &TestApp, by: &User, code: &str, amount: i64) -> CompanyBudget {
    let entity = entity(app, "GMS").await;
    let budget = budget_lines::create(
        app.state(),
        NewBudget {
            legal_entity_id: entity.id,
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

async fn approved_budget(
    app: &TestApp,
    by: &User,
    admin: &User,
    code: &str,
    amount: i64,
) -> CompanyBudget {
    let budget = draft_budget(app, by, code, amount).await;
    app.acting_as(by);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(app, &budget).await;
    approvals::approve(app.state(), request.id, admin, "")
        .await
        .unwrap()
        .expect("the approval runs");
    CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap()
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

/// AC8 (Budget and drawings), the budget: a monthly budget prepared by the
/// accountant needs the MD and not its preparer; the accountant owns every
/// category, the 09 ones included (visa and insurance, accommodation, office);
/// an expense line reaching 100% of its month's line warns the owner role and
/// the accountant once.
#[renox::test]
async fn the_md_approves_the_budget_and_a_line_reaching_its_month_warns_the_accountant_once() {
    let app = boot_seeded().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let bilal = person(&app, "Bilal", ACCOUNTANT).await;
    let md = person(&app, "Alia", ADMIN).await;
    for code in [
        "visa_insurance",
        "accommodation",
        "office",
        "salaries_staff",
        "transport_fuel",
        "rent_utilities",
    ] {
        let category = BudgetCategory::by_code(app.db(), code)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the category {code}"));
        assert_eq!(category.owner_role.as_deref(), Some(ACCOUNTANT), "{code}");
    }

    let budget = draft_budget(&app, &amal, "rent_utilities", 500_000).await;
    app.acting_as(&amal);
    app.htmx()
        .post(&format!("/budgets/{}/submit", budget.id), &[])
        .await
        .assert_redirect(&format!("/budgets/{}", budget.id));
    let request = pending(&app, &budget).await;
    let own = approvals::approve(app.state(), request.id, &amal, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    approvals::approve(app.state(), request.id, &md, "")
        .await
        .unwrap();
    let approved = CompanyBudget::find_or_404(app.db(), budget.id)
        .await
        .unwrap();
    assert_eq!(approved.status, BudgetStatus::Approved);

    let vendor = company(&app, "Dubai Utilities LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    let first = bill(&app, &amal, &code, "UT-1", "4000", "utilities").await;
    approve_bill(&app, &bilal, &first).await;
    assert_eq!(support::count_messages(&app, VARIANCE).await, 0);
    let second = bill(&app, &amal, &code, "UT-2", "1000", "utilities").await;
    approve_bill(&app, &bilal, &second).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1);
    assert_eq!(messages_to(&app, VARIANCE, &bilal).await.len(), 1);
    let third = bill(&app, &amal, &code, "UT-3", "300", "utilities").await;
    approve_bill(&app, &bilal, &third).await;
    assert_eq!(messages_to(&app, VARIANCE, &amal).await.len(), 1, "once");
}

/// AC8, the drawings: an expense with the category *owner drawings* is left out
/// of the operating cost and profit reports, and the user who records it cannot
/// approve it (`FIN.ACL.21`); the MD does.
#[renox::test]
async fn an_owner_drawing_is_the_mds_to_approve_and_is_not_operating_cost() {
    let app = boot_seeded().await;
    let amal = person(&app, "Amal", ACCOUNTANT).await;
    let md = person(&app, "Alia", ADMIN).await;
    let vendor = company(&app, "Villa Decor LLC").await;
    let code = wl_parties::Party::find_or_404(app.db(), vendor.0)
        .await
        .unwrap()
        .code;
    approved_budget(&app, &amal, &md, "rent_utilities", 500_000).await;

    let drawing = bill(&app, &amal, &code, "VD-1", "2000", "owner_drawings").await;
    let ordinary = bill(&app, &amal, &code, "VD-2", "1000", "other").await;
    approve_bill(&app, &amal, &ordinary).await;

    let row = DrawingApproval::for_source(app.db(), &drawing.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(row.status, DrawingStatus::Requested);
    assert_eq!(row.requested_by, Some(amal.id));
    let request_id = row.approval_request_id.expect("an approval request");
    app.acting_as(&amal);
    app.htmx()
        .post(
            &format!("/supplier-bills/{}/transitions/approve", drawing.id),
            &[("reason", "")],
        )
        .await
        .assert_status(422);
    assert!(
        approvals::approve(app.state(), request_id, &amal, "")
            .await
            .is_err(),
        "the requester cannot approve their drawing"
    );
    approvals::approve(app.state(), request_id, &md, "")
        .await
        .unwrap()
        .expect("the approval runs");
    approve_bill(&app, &amal, &drawing).await;

    let gms = entity(&app, "GMS").await;
    let config = host::invoicing_for("manpower", &["GMS".to_owned()])
        .config()
        .clone();
    let report = budget_report::build(
        app.db(),
        &config,
        &ReportQuery {
            entities: vec![gms.id],
            month: day(0),
            ytd: false,
            drawings_lines: true,
            uncategorised: "Uncategorised".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.drawings_total().1, 210_000);
    assert_eq!(
        report.expense_total().1,
        100_000,
        "the drawing is not operating cost"
    );
}
