//! The actions of the field PWA and the back office on attendance (wl-attendance
//! §5.1, §5.2, issue #178): supervisor marking, the employee's own check-in
//! and check-out, each behind its permission at the location.

mod support;

use wl_attendance::{AttendanceMethod, AttendanceRecord, AttendanceStatus, WlAttendance};
use wl_kernel::Subject;

use support::{at, boot, boot_with, nov};

/// The form of a marking by a supervisor.
fn marking(employee: i64, location: i64) -> Vec<(&'static str, String)> {
    vec![
        ("employee_id", employee.to_string()),
        ("location_id", location.to_string()),
        ("check_in", "2026-11-02T07:00".to_owned()),
        ("check_out", "2026-11-02T15:00".to_owned()),
        ("note", "paper list".to_owned()),
    ]
}

/// Posts `form` as the signed-in user.
async fn post(
    t: &support::Booted,
    path: &str,
    form: &[(&str, String)],
) -> renox::testing::TestResponse {
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t.app.htmx().post(path, &form).await
}

#[renox::test]
async fn att01_a_supervisor_with_the_permission_marks_and_others_cannot() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let nobody = t.user_with("Nadia", "reviewer").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(2), async {
        t.app.acting_as(&nobody);
        post(&t, "/attendance/mark", &marking(a.0, site1.id))
            .await
            .assert_forbidden();
        assert!(t.records_of(a).await.is_empty());

        t.app.acting_as(&boss);
        post(&t, "/attendance/mark", &marking(a.0, site1.id))
            .await
            .assert_ok();
        let saved = t.records_of(a).await;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].check_in, at(2, 7, 0));
        assert_eq!(saved[0].check_out, Some(at(2, 15, 0)));
        assert_eq!(saved[0].recorded_by, Some(boss.id));
        assert_eq!(saved[0].note, "paper list");
        assert_eq!(saved[0].method, AttendanceMethod::Supervisor);
    })
    .await;
}

#[renox::test]
async fn att01_a_marking_with_a_bad_time_or_an_old_day_is_refused() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(4), async {
        t.app.acting_as(&boss);
        let mut form = marking(a.0, site1.id);
        form[2].1 = "yesterday morning".to_owned();
        post(&t, "/attendance/mark", &form).await.assert_status(422);
        // 2 November is outside the one-day window on 4 November.
        post(&t, "/attendance/mark", &marking(a.0, site1.id))
            .await
            .assert_status(422);
        assert!(t.records_of(a).await.is_empty());
    })
    .await;
}

/// A supervisor holding the permission in one project only may mark at that
/// project's locations only.
#[renox::test]
async fn att01_the_permission_is_checked_in_the_scope_of_the_location() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let one = support::project(1);
    let two = support::project(2);
    let site1 = t
        .location_of(wl_attendance::WorkLocation::named("Site 1").in_scope(&one))
        .await;
    let site2 = t
        .location_of(wl_attendance::WorkLocation::named("Site 2").in_scope(&two))
        .await;
    let boss = {
        let email = "scoped@example.com";
        let user = renox::prelude::User::register(t.db(), "Scoped", email, "password123")
            .await
            .unwrap();
        user.assign_role_in(
            t.db(),
            "supervisor",
            &renox::auth::permissions::Scope::new(&one.subject_type, one.subject_id),
        )
        .await
        .unwrap();
        user
    };
    t.on(nov(2), async {
        t.app.acting_as(&boss);
        post(&t, "/attendance/mark", &marking(a.0, site2.id))
            .await
            .assert_forbidden();
        post(&t, "/attendance/mark", &marking(a.0, site1.id))
            .await
            .assert_ok();
        assert_eq!(t.records_of(a).await.len(), 1);
        let _: Subject = two;
    })
    .await;
}

#[renox::test]
async fn att01_an_employee_checks_in_and_out_for_themselves() {
    let t = boot().await;
    let worker = t.user_with("Wasim", "worker").await;
    let other = t.user_with("Omar", "worker").await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    t.staff.account(worker.id, a);
    let site1 = t.location("Site 1").await;
    t.on(nov(2), async {
        // An account with no employee record cannot check in.
        t.app.acting_as(&other);
        post(
            &t,
            "/attendance/check-in",
            &[("location_id", site1.id.to_string())],
        )
        .await
        .assert_status(422);

        t.app.acting_as(&worker);
        post(
            &t,
            "/attendance/check-in",
            &[("location_id", site1.id.to_string())],
        )
        .await
        .assert_ok();
        let record: AttendanceRecord = t.records_of(a).await.remove(0);
        assert_eq!(record.method, AttendanceMethod::SelfCheckIn);
        assert_eq!(record.recorded_by, Some(worker.id));
        assert_eq!(record.status, AttendanceStatus::Recorded);

        // A second check-in is a duplicate.
        post(
            &t,
            "/attendance/check-in",
            &[("location_id", site1.id.to_string())],
        )
        .await
        .assert_status(422);
        assert_eq!(t.records_of(a).await.len(), 1);

        // Another employee cannot check the worker out; the worker can.
        let path = format!("/attendance/records/{}/check-out", record.id);
        t.app.acting_as(&other);
        post(&t, &path, &[]).await.assert_forbidden();
        t.app.acting_as(&worker);
        post(&t, &path, &[]).await.assert_ok();
        assert!(t.reload(&record).await.check_out.is_some());
        // A supervisor of the location could have done it as well.
        let _ = boss;
    })
    .await;
}

#[renox::test]
async fn att01_the_module_builds_with_its_own_settings() {
    let t = boot_with(WlAttendance::new(), "").await;
    assert!(t.records_of(t.employee(1, &[])).await.is_empty());
}
