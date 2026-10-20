//! The attendance record (wl-attendance §4 `record`, issue #178): who may be
//! recorded, the late-entry window of `HRM.ACL.01`, the events, and the
//! employee's own check-in and check-out.

mod support;

use renox::prelude::*;
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceMethod, AttendanceStatus};

use support::{at, boot, nov};

/// The message of a refusal (a 422).
fn refusal(error: Error) -> String {
    match error {
        Error::Status(status, message) => {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
            message
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

/// A supervisor's marking is saved `recorded`, with who made it, and
/// `AttendanceRecorded` follows.
#[renox::test]
async fn att01_a_supervisor_marks_an_employee_present() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Recorded);
        assert_eq!(record.method, AttendanceMethod::Supervisor);
        assert_eq!(record.date, nov(2));
        assert_eq!(record.recorded_by, Some(boss.id));
        assert_eq!(record.check_out, Some(at(2, 15, 0)));
        assert!(t.flags_of(&record).await.is_empty());
        assert!(t.history_of(&record).await.is_empty(), "no transition yet");

        let events = t.recorded.all();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].record.0, record.id);
        assert_eq!(events[0].employee, a);
        assert_eq!(events[0].location.to_string(), format!("attendance.work_location:{}", site1.id));
        assert_eq!(events[0].date, nov(2));
        assert_eq!(events[0].method, "supervisor");
        assert!(events[0].flags.is_empty());
        assert!(t.flagged.all().is_empty());
    })
    .await;
}

/// `HRM.ACL.01`: today or one day back; older days are refused and nothing is
/// saved, and so is a day that has not come.
#[renox::test]
async fn att01_the_late_entry_window_is_one_day() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(4), async {
        let yesterday = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 7, 0), Some(at(3, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(yesterday.date, nov(3));

        let old = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap_err();
        let message = refusal(old);
        assert!(message.contains("up to 1 day(s)"), "{message}");
        assert!(message.contains("2026-11-02"), "{message}");

        let early = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(5, 7, 0), Some(at(5, 15, 0)), boss.id),
        )
        .await
        .unwrap_err();
        assert!(refusal(early).contains("has not come"));
        assert_eq!(t.records_of(a).await.len(), 1, "only the day inside the window");
    })
    .await;
}

/// The window is a setting.
#[renox::test]
async fn att01_the_window_follows_the_setting() {
    let t = support::boot_with(wl_attendance::WlAttendance::new().late_entry_days(3), "").await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(5), async {
        service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        let old = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(1, 7, 0), Some(at(1, 15, 0)), boss.id),
        )
        .await
        .unwrap_err();
        assert!(refusal(old).contains("up to 3 day(s)"));
    })
    .await;
}

/// An employee who is on leave, gone or unknown cannot be recorded; nor can
/// a check-out before the check-in.
#[renox::test]
async fn att01_only_an_employee_at_work_with_sound_times_is_recorded() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(2), async {
        t.staff.change(a, |p| p.on_leave = true);
        let on_leave = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), None, boss.id),
        )
        .await
        .unwrap_err();
        assert!(refusal(on_leave).contains("not at work"));
        t.staff.change(a, |p| {
            p.on_leave = false;
            p.listed = false;
        });
        let gone = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), None, boss.id),
        )
        .await
        .unwrap_err();
        assert!(refusal(gone).contains("not at work"));
        let unknown = service::record(
            t.state(),
            NewRecord::supervisor(wl_kernel::workforce::EmployeeId(99), site1.id, at(2, 7, 0), None, boss.id),
        )
        .await
        .unwrap_err();
        assert!(refusal(unknown).contains("not at work"));

        t.staff.change(a, |p| p.listed = true);
        let backwards = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 15, 0), Some(at(2, 7, 0)), boss.id),
        )
        .await
        .unwrap_err();
        assert!(refusal(backwards).contains("after the check-in"));
        let unknown_site = service::record(
            t.state(),
            NewRecord::supervisor(a, 999, at(2, 7, 0), None, boss.id),
        )
        .await;
        assert!(matches!(unknown_site, Err(Error::NotFound)));
        assert!(t.records_of(a).await.is_empty());
        assert!(t.recorded.all().is_empty());
    })
    .await;
}

/// Self check-in at a second location is refused while the employee is
/// checked in at the first: the message names it, and no record is created.
#[renox::test]
async fn att01_self_check_in_elsewhere_is_refused_until_the_employee_checks_out() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(2), async {
        let first = service::check_in_self(t.state(), a, site1.id, at(2, 7, 0), Some(7))
            .await
            .unwrap();
        assert_eq!(first.method, AttendanceMethod::SelfCheckIn);
        assert_eq!(first.check_out, None);

        let elsewhere = service::check_in_self(t.state(), a, site2.id, at(2, 9, 0), Some(7))
            .await
            .unwrap_err();
        let message = refusal(elsewhere);
        assert!(message.contains("Site 1"), "{message}");
        assert_eq!(t.records_of(a).await.len(), 1, "no record was created");

        // Again at the same place: a duplicate.
        let again = service::check_in_self(t.state(), a, site1.id, at(2, 8, 0), Some(7))
            .await
            .unwrap_err();
        assert_eq!(refusal(again), "You are already checked in here.");
        assert_eq!(t.records_of(a).await.len(), 1);
        assert_eq!(t.recorded.all().len(), 1);
    })
    .await;
}

/// After the check-out the employee may check in at the second location; the
/// record is `recorded` with no flag.
#[renox::test]
async fn att01_after_checking_out_the_employee_checks_in_elsewhere() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(2), async {
        let first = service::check_in_self(t.state(), a, site1.id, at(2, 7, 0), Some(7))
            .await
            .unwrap();
        let closed = service::check_out(t.state(), first.id, at(2, 8, 30))
            .await
            .unwrap();
        assert_eq!(closed.check_out, Some(at(2, 8, 30)));
        assert_eq!(closed.status, AttendanceStatus::Recorded);

        let second = service::check_in_self(t.state(), a, site2.id, at(2, 9, 0), Some(7))
            .await
            .unwrap();
        assert_eq!(second.status, AttendanceStatus::Recorded);
        assert_eq!(second.location_id, site2.id);
        assert!(t.flags_of(&second).await.is_empty());
        assert!(t.flags_of(&closed).await.is_empty());
        assert!(t.flagged.all().is_empty());
        // A check-in, a check-out and a check-in each emit the event.
        assert_eq!(t.recorded.all().len(), 3);
    })
    .await;
}

/// A check-out needs a record that is open, not locked and before the time.
#[renox::test]
async fn att01_a_check_out_is_refused_for_a_locked_or_closed_record() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(2), async {
        let locked = t
            .fixture(a, &site1, at(2, 7, 0), Some(at(2, 10, 0)), AttendanceStatus::Locked)
            .await;
        let refused = service::check_out(t.state(), locked.id, at(2, 15, 0))
            .await
            .unwrap_err();
        assert!(refusal(refused).contains("does not change"));

        let open = service::check_in_self(t.state(), a, site1.id, at(2, 16, 0), None)
            .await
            .unwrap();
        let early = service::check_out(t.state(), open.id, at(2, 15, 0))
            .await
            .unwrap_err();
        assert!(refusal(early).contains("after the check-in"));
        service::check_out(t.state(), open.id, at(2, 17, 0))
            .await
            .unwrap();
        let twice = service::check_out(t.state(), open.id, at(2, 18, 0))
            .await
            .unwrap_err();
        assert!(refusal(twice).contains("check-out already"));
    })
    .await;
}
