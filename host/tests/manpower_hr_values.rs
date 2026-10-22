//! BL-MPW-21 (#435), `HRM.ACL.05`, `HRM.ACL.06`, `HRM.ACL.04`, `HRM.RUL.02`:
//! the 09 HR values at work in the seeded host with the real wl-hr, wl-notify
//! and wl-attendance, not read back as constants: a changed pay account
//! cools for the seeded 5 days and the SMS goes to the number on record
//! before the request; a recovery of 25 % of the wage is refused when the
//! payroll run is approved; the payroll cut-off of the seeded config decides
//! which attendance is paid provisionally.

mod manpower_support;
mod support;

use host::roles::{HR, OWNER};
use manpower_support::{
    DeliveringSms, boot_seeded_with, draft_run, run_transition, salary, worker_with_mobile,
};
use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::person;
use wl_hr::pay_account::{self, PayAccount, PayAccountStatus};
use wl_hr::{Adjustment, AdjustmentKind, PayrollLine, PayrollRunStatus};
use wl_kernel::documents::today;
use wl_notify::Message;
use wl_parties::{ContactPoint, ContactType};

const OLD_NUMBER: &str = "+971501110001";
const NEW_NUMBER: &str = "+971502220002";
const IBAN: &str = "AE550331234567890121234";

async fn agency() -> (TestApp, DeliveringSms) {
    let sms = DeliveringSms::default();
    let tuned = sms.clone();
    let app = boot_seeded_with(host::Tuning::default().notify(move |n| n.provider(tuned))).await;
    (app, sms)
}

async fn account_of(app: &TestApp, employee: i64) -> PayAccount {
    PayAccount::of_employee(app.db(), employee)
        .await
        .unwrap()
        .pop()
        .expect("the pay account")
}

/// AC5 (HR), `HRM.ACL.05`: HR asks for a new pay account for W and the MD
/// approves it; W's number is changed on record afterwards, and the SMS still
/// goes to the number W had when the request was made. The account cools for
/// the seeded 5 days: it is still cooling after 4 and `active` after 5, when
/// the old account retires.
#[renox::test]
async fn a_changed_pay_account_cools_for_five_days_and_the_sms_goes_to_the_old_number() {
    let (app, sms) = agency().await;
    let hr = person(&app, "Hana", HR).await;
    let md = person(&app, "Mazen", OWNER).await;
    let w = worker_with_mobile(&app, "Wasim Khan", OLD_NUMBER).await;
    let old = pay_account_fixture(&app, w.0).await;

    app.acting_as(&hr);
    app.htmx()
        .post(
            &format!("/hr/employees/{}/pay-accounts", w.0),
            &[
                ("channel", "bank_account"),
                ("account_number", IBAN),
                ("routing_code", "123456789"),
                ("holder_name", "Wasim Khan"),
            ],
        )
        .await
        .assert_status(303);
    let new = account_of(&app, w.0).await;
    assert_eq!(new.status, PayAccountStatus::PendingApproval);
    assert_eq!(new.cooling_days, 5, "the seeded cooling is kept on the row");

    // W's number changes on record after the request: the notice must not use it.
    ContactPoint::query()
        .where_(ContactPoint::CONTACT_TYPE.eq(ContactType::Mobile))
        .update(app.db(), &[("value", &NEW_NUMBER.to_owned())])
        .await
        .unwrap();

    app.acting_as(&md);
    app.htmx()
        .post(
            &format!("/hr/pay-accounts/{}/transitions/approve", new.id),
            &[("reason", "")],
        )
        .await
        .assert_status(303);
    app.run_all_jobs().await;
    let cooling = PayAccount::find_or_404(app.db(), new.id).await.unwrap();
    assert_eq!(cooling.status, PayAccountStatus::Cooling);
    assert_eq!(cooling.cooling_ends_on, Some(today() + Days::new(5)));
    assert_eq!(
        sms.numbers(),
        [OLD_NUMBER],
        "the SMS goes to the old number"
    );
    let notice = Message::query()
        .where_(Message::TYPE_KEY.eq("hr.pay_account_change"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the change notice");
    assert_eq!(notice.address.as_deref(), Some(OLD_NUMBER));
    assert!(
        notice.rendered_body.contains("1234"),
        "{}",
        notice.rendered_body
    );

    for (days, status, old_status) in [
        (4, PayAccountStatus::Cooling, PayAccountStatus::Active),
        (1, PayAccountStatus::Active, PayAccountStatus::Retired),
    ] {
        app.travel(std::time::Duration::from_secs(days * 24 * 3600));
        app.at_travelled_time(async {
            app.run_scheduled(pay_account::TASK).await.unwrap();
        })
        .await;
        let now = PayAccount::find_or_404(app.db(), new.id).await.unwrap();
        assert_eq!(now.status, status, "{days} more days");
        let before = PayAccount::find_or_404(app.db(), old.id).await.unwrap();
        assert_eq!(before.status, old_status, "{days} more days");
    }
}

/// The account W is paid into now.
async fn pay_account_fixture(app: &TestApp, employee: i64) -> PayAccount {
    let mut account = PayAccount {
        employee_id: employee,
        channel: "bank_account".into(),
        routing_code: Some("123456789".into()),
        account_number: "AE100261234567890124321".into(),
        last4: "4321".into(),
        holder_name: "Wasim Khan".into(),
        cooling_days: 5,
        status: PayAccountStatus::Active,
        ..PayAccount::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    account
        .insert_in_state(&mut tx, PayAccountStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    account
}

/// AC5, `HRM.ACL.06`: W earns AED 3,000. A recovery of AED 750 (25 %) is above
/// the overpayment cap of 20 %: HR calculates the November run and the MD's
/// approval is refused, nothing is trimmed. A recovery of AED 600 (20 %) is
/// approved.
#[renox::test]
async fn a_recovery_of_25_percent_is_refused_when_the_run_is_approved() {
    for (recovery, approved) in [(75_000, false), (60_000, true)] {
        let (app, _sms) = agency().await;
        let hr = person(&app, "Hana", HR).await;
        let md = person(&app, "Mazen", OWNER).await;
        let w = worker_with_mobile(&app, "Wasim Khan", OLD_NUMBER).await;
        salary(&app, w, 300_000).await;
        Adjustment::create(
            app.db(),
            Adjustment {
                employee_id: w.0,
                kind: AdjustmentKind::OverpaymentRecovery,
                amount: recovery,
                period: "2026-11".into(),
                ..Adjustment::default()
            },
        )
        .await
        .unwrap();
        let run = draft_run(&app, "2026-11").await;

        run_transition(&app, &hr, &run, "calculate")
            .await
            .assert_status(303);
        let answer = run_transition(&app, &md, &run, "approve").await;

        let after = wl_hr::PayrollRun::find_or_404(app.db(), run.id)
            .await
            .unwrap();
        if approved {
            answer.assert_status(303);
            assert_eq!(after.status, PayrollRunStatus::Approved);
        } else {
            answer.assert_status(422);
            assert_eq!(after.status, PayrollRunStatus::Calculated);
            let problems = wl_hr::caps::problems(app.db(), run.id).await.unwrap();
            assert_eq!(problems.len(), 1);
            assert_eq!(problems[0].amount, recovery);
            let kept = Adjustment::query()
                .where_(Adjustment::EMPLOYEE_ID.eq(w.0))
                .first(app.db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(kept.amount, recovery, "refused, not trimmed");
        }
        let line = PayrollLine::query()
            .where_(PayrollLine::RUN_ID.eq(run.id))
            .first(app.db())
            .await
            .unwrap()
            .expect("W's line");
        assert_eq!(line.fixed_wage, 300_000);
    }
}

/// `HRM.RUL.02` with the seeded cut-off: the November run calculated by HR
/// pays W's attendance accepted up to the cut-off provisionally (the client's
/// timesheet is not approved); the day after the cut-off waits.
#[renox::test]
async fn the_seeded_cut_off_decides_which_attendance_is_paid_provisionally() {
    use wl_attendance::timesheet::ApproverKind;
    use wl_attendance::{AttendanceRecord, Timesheet, TimesheetStatus, WorkLocation};
    use wl_kernel::Subject;

    let (app, _sms) = agency().await;
    let hr = person(&app, "Hana", HR).await;
    let w = worker_with_mobile(&app, "Wasim Khan", OLD_NUMBER).await;
    salary(&app, w, 300_000).await;
    let site = WorkLocation::create(app.db(), WorkLocation::named("Tower A"))
        .await
        .unwrap();
    let calendar = wl_kernel::BusinessCalendar::default();
    let nov = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    // Four accepted days of 8 hours before the cut-off, one the day after it.
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

    let run = draft_run(&app, "2026-11").await;
    run_transition(&app, &hr, &run, "calculate")
        .await
        .assert_status(303);
    let line = PayrollLine::query()
        .where_(PayrollLine::RUN_ID.eq(run.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("W's line");
    assert_eq!(
        line.provisional_minutes,
        4 * 8 * 60,
        "the cut-off of the seeded config is the 25th"
    );
}
