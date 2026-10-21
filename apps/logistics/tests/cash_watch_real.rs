//! BL-LOG-13 (#311), `LOG.RUL.13`, `HRM.ABS.01`, `HRM.ATT.01` (open item
//! L-20): the 48-hour escalation of a driver's cash with the real wl-hr and
//! wl-attendance. The absence note is an absence case on the driver's real HR
//! record; a check-in is a real attendance record that `checked_in_since`
//! answers from. The delivery module and the cash custody are real too.

mod support;

use logistics::cash_watch::{self, CashWatch, Watched};
use logistics::permissions as perm;
use renox::auth::permissions;
use renox::chrono::Duration;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_attendance::{AttendanceMethod, AttendanceRecord, AttendanceStatus, WorkLocation};
use wl_hr::{AbsenceCase, Employee, EmployeeStatus};
use wl_kernel::custody::{HandoverSummary, custody};
use wl_kernel::delivery::RunId;
use wl_payments::permissions as pperm;

use support::{SOUQ, World, boot_with_labour};

/// The driver as an employee of the real wl-hr, linked to his user.
async fn driver_employee(w: &World) -> Employee {
    let mut employee = Employee {
        party_id: 60,
        user_id: Some(w.driver.id),
        job_role: "driver".into(),
        job_title: "Driver".into(),
        hire_date: renox::chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        status: EmployeeStatus::Active,
        ..Employee::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    employee
        .insert_in_state(&mut tx, EmployeeStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    employee
}

/// The operations manager and the company admin.
async fn managers(w: &World) {
    permissions::define_role(
        w.db(),
        perm::OPERATIONS_MANAGER_ROLE,
        &[perm::SHIPMENTS_VIEW],
    )
    .await
    .unwrap();
    let mona = w
        .user_with(
            "Mona",
            &[perm::SHIPMENTS_VIEW, pperm::CASH_VIEW, pperm::CASH_COUNT],
        )
        .await;
    mona.assign_role(w.db(), perm::OPERATIONS_MANAGER_ROLE)
        .await
        .unwrap();
    permissions::define_role(w.db(), wl_kernel::COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let amira = w.user_with("Amira", &[perm::SHIPMENTS_VIEW]).await;
    amira
        .assign_role(w.db(), wl_kernel::COMPANY_ADMIN_ROLE)
        .await
        .unwrap();
}

/// A COD shipment of AED 1,800 delivered on a run that returned with the
/// cash not handed over: the run and the instant the clock starts.
async fn returned_with_cash(w: &World) -> (i64, DateTime) {
    let shipment = w
        .book_with(&w.dispatcher, SOUQ, "ORD-1", &[("cod", "1800")])
        .await;
    let (_out, delivery) = w.out_for_delivery(&shipment).await;
    w.deliver_stop(&delivery, "1800.00")
        .await
        .assert_status(303);
    let run = w.delivery_of(&shipment).await.run_id.expect("on a run");
    w.post(&w.driver, &format!("/driver/runs/{run}/return"), &[])
        .await
        .assert_ok();
    let handover: HandoverSummary = custody(w.state())
        .handover_of(w.db(), w.driver.id, &RunId(run).subject())
        .await
        .unwrap()
        .expect("the handover");
    (run, handover.ended_at.expect("the return ended the period"))
}

/// AC3: after 48 hours the driver's HR record has an absence case, opened by
/// the escalation, for the run, with the day of the run as the last day he was
/// known to have worked.
#[renox::test]
async fn the_absence_note_is_a_real_absence_case_on_the_drivers_hr_record() {
    let w = boot_with_labour().await;
    managers(&w).await;
    let employee = driver_employee(&w).await;
    let (run, start) = returned_with_cash(&w).await;
    assert!(
        AbsenceCase::open_of(w.db(), employee.id)
            .await
            .unwrap()
            .is_empty()
    );

    let done = cash_watch::run(w.state(), start + Duration::hours(48))
        .await
        .unwrap();
    assert_eq!(done.absence_notes, 1);
    let cases = AbsenceCase::open_of(w.db(), employee.id).await.unwrap();
    assert_eq!(cases.len(), 1, "one absence case on his record");
    assert_eq!(cases[0].case_type, "delivery.run");
    assert_eq!(cases[0].case_id, run);
    assert_eq!(cases[0].opened_on, wl_kernel::documents::today());

    // Run again: the escalation does not open a second case.
    let again = cash_watch::run(w.state(), start + Duration::hours(72))
        .await
        .unwrap();
    assert_eq!(again, Watched::default());
    assert_eq!(
        AbsenceCase::open_of(w.db(), employee.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// AC7: a real attendance record whose check-in follows the clock start makes
/// `checked_in_since` true, so the escalation tells the admin and asks for the
/// count but writes no absence case.
#[renox::test]
async fn a_real_check_in_after_the_start_means_no_absence_case() {
    let w = boot_with_labour().await;
    managers(&w).await;
    let employee = driver_employee(&w).await;
    let (_run, start) = returned_with_cash(&w).await;

    let location = WorkLocation::create(
        w.db(),
        WorkLocation {
            name: "Depot A".into(),
            ..WorkLocation::default()
        },
    )
    .await
    .unwrap();
    let check_in = start + Duration::hours(12) + Duration::minutes(30);
    let mut record = AttendanceRecord {
        employee_id: employee.id,
        location_id: location.id,
        date: wl_kernel::documents::dubai_date(check_in),
        check_in,
        method: AttendanceMethod::Supervisor,
        recorded_at: check_in,
        status: AttendanceStatus::Recorded,
        ..AttendanceRecord::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    record
        .insert_in_state(&mut tx, AttendanceStatus::Recorded, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let done = cash_watch::run(w.state(), start + Duration::hours(48))
        .await
        .unwrap();
    assert_eq!(done.admin_notices, 1);
    assert_eq!(done.absence_notes, 0);
    assert!(
        AbsenceCase::open_of(w.db(), employee.id)
            .await
            .unwrap()
            .is_empty()
    );
    let watch = CashWatch::query().first(w.db()).await.unwrap().unwrap();
    assert_eq!(watch.absence_skipped.as_deref(), Some("checked_in"));
}

/// A check-in before the clock start does not count: the note is written.
#[renox::test]
async fn a_check_in_before_the_start_does_not_excuse_the_driver() {
    let w = boot_with_labour().await;
    managers(&w).await;
    let employee = driver_employee(&w).await;
    let (_run, start) = returned_with_cash(&w).await;
    let location = WorkLocation::create(
        w.db(),
        WorkLocation {
            name: "Depot A".into(),
            ..WorkLocation::default()
        },
    )
    .await
    .unwrap();
    let check_in = start - Duration::hours(6);
    let mut record = AttendanceRecord {
        employee_id: employee.id,
        location_id: location.id,
        date: wl_kernel::documents::dubai_date(check_in),
        check_in,
        method: AttendanceMethod::Supervisor,
        recorded_at: check_in,
        status: AttendanceStatus::Recorded,
        ..AttendanceRecord::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    record
        .insert_in_state(&mut tx, AttendanceStatus::Recorded, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let done = cash_watch::run(w.state(), start + Duration::hours(48))
        .await
        .unwrap();
    assert_eq!(done.absence_notes, 1);
    assert_eq!(
        AbsenceCase::open_of(w.db(), employee.id)
            .await
            .unwrap()
            .len(),
        1
    );
}
