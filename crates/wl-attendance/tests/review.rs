//! The review of attendance (wl-attendance §4 `accept`, `correct`, `reject`,
//! `lock`; `HRM.ACL.09`, issue #182): a record held by `overtime_cap`,
//! `permit_invalid` or `overlap` is accepted or corrected only by a reviewer,
//! with a reason, and not by the person who marked it. Every action goes
//! through its route as the signed-in user.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::json;
use renox::testing::TestResponse;
use wl_attendance::review;
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceFlag, AttendanceRecord, AttendanceStatus, FlagType, WorkLocation};
use wl_kernel::workforce::EmployeeId;

use support::{Booted, at, boot, dubai, nov};

/// `user` posts `form` to `/attendance/records/{id}/{action}`.
async fn act(
    t: &Booted,
    user: &User,
    record: &AttendanceRecord,
    action: &str,
    form: &[(&str, &str)],
) -> TestResponse {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(&format!("/attendance/records/{}/{action}", record.id), form)
        .await
}

/// A record of 16 hours (06:00 to 22:00), over the overtime cap, marked by
/// `by`: `flagged` `overtime_cap`.
async fn over_cap(t: &Booted, a: EmployeeId, site: &WorkLocation, by: i64) -> AttendanceRecord {
    let record = service::record(
        t.state(),
        NewRecord::supervisor(a, site.id, at(2, 6, 0), Some(at(2, 22, 0)), by),
    )
    .await
    .unwrap();
    assert_eq!(record.status, AttendanceStatus::Flagged);
    assert_eq!(t.flags_of(&record).await, ["overtime_cap"]);
    record
}

/// A record flagged `overtime_cap` marked by supervisor B: B cannot accept it,
/// even holding `attendance.review`; reviewer C accepts with a reason, which
/// is in the history, and not without one.
#[renox::test]
async fn att05_the_marker_cannot_accept_and_a_reviewer_needs_a_reason() {
    let t = boot().await;
    let b = t.user_with("Bashir", "lead").await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = over_cap(&t, a, &site, b.id).await;

        act(&t, &b, &record, "accept", &[("reason", "emergency repair")])
            .await
            .assert_status(422)
            .assert_see("HRM.ACL.09");
        assert_eq!(t.reload(&record).await.status, AttendanceStatus::Flagged);

        act(&t, &c, &record, "accept", &[("reason", "  ")])
            .await
            .assert_status(422);
        act(&t, &c, &record, "accept", &[])
            .await
            .assert_status(422);
        assert_eq!(t.reload(&record).await.status, AttendanceStatus::Flagged);

        let reason = "emergency repair, Art. exception";
        act(&t, &c, &record, "accept", &[("reason", reason)])
            .await
            .assert_ok();
        let accepted = t.reload(&record).await;
        assert_eq!(accepted.status, AttendanceStatus::Accepted);
        let accept = t
            .history_of(&accepted)
            .await
            .into_iter()
            .find(|h| h.0 == "accept")
            .expect("the transition is in the history");
        assert_eq!(accept.1, Some(c.id));
        assert_eq!(accept.2.as_deref(), Some(reason));

        // The flag stays as history, settled with the reason.
        let flags = AttendanceFlag::of_record(t.db(), record.id).await.unwrap();
        assert_eq!(flags.len(), 1);
        assert!(flags[0].resolved_at.is_some());
        assert_eq!(flags[0].resolved_by, Some(c.id));
        assert_eq!(flags[0].resolution.as_deref(), Some(reason));
        assert!(
            AttendanceFlag::open_of_record(t.db(), record.id)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
}

/// Only a holder of `attendance.review` at the location accepts or rejects.
#[renox::test]
async fn att05_accept_and_reject_need_the_review_permission() {
    let t = boot().await;
    let b = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = over_cap(&t, a, &site, b.id).await;
        act(&t, &b, &record, "accept", &[("reason", "ok")])
            .await
            .assert_forbidden();
        act(&t, &b, &record, "reject", &[("reason", "ok")])
            .await
            .assert_forbidden();
        assert_eq!(t.reload(&record).await.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// The employee the record is about never accepts it, with the permission
/// and a reason (self check-in records: the marker is the employee).
#[renox::test]
async fn att05_an_employee_never_accepts_their_own_record() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    t.staff.account(c.id, a);
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = over_cap(&t, a, &site, boss.id).await;
        act(&t, &c, &record, "accept", &[("reason", "mine")])
            .await
            .assert_status(422)
            .assert_see("HRM.ACL.09");
        assert_eq!(t.reload(&record).await.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// A record flagged only `midday_break` marked by B: B (with
/// `attendance.review`) may accept it, with no reason.
#[renox::test]
async fn att05_a_flag_that_holds_nothing_may_be_accepted_by_the_marker() {
    let t = boot().await;
    let b = t.user_with("Bashir", "lead").await;
    let a = t.employee(1, &[]);
    let site = t
        .location_of(WorkLocation::named("Site 1").outdoors())
        .await;
    let july = NaiveDate::from_ymd_opt(2027, 7, 12).unwrap();
    t.on(july, async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, dubai(july, 7, 0), Some(dubai(july, 17, 0)), b.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&record).await, ["midday_break"]);
        act(&t, &b, &record, "accept", &[]).await.assert_ok();
        assert_eq!(t.reload(&record).await.status, AttendanceStatus::Accepted);
    })
    .await;
}

/// Two records flagged `overlap`: reviewer C cannot accept one while both
/// still overlap (the refusal names the other's location and times); after C
/// rejects the other, accept succeeds.
#[renox::test]
async fn att05_an_overlap_is_accepted_only_when_the_hours_no_longer_overlap() {
    let t = boot().await;
    let x = t.user_with("Xavier", "supervisor").await;
    let y = t.user_with("Yasmin", "supervisor").await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(2), async {
        let r1 = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), Some(at(2, 15, 0)), x.id),
        )
        .await
        .unwrap();
        let r2 = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(2, 13, 0), Some(at(2, 17, 0)), y.id),
        )
        .await
        .unwrap();
        let (r1, r2) = (t.reload(&r1).await, t.reload(&r2).await);
        assert_eq!(r1.status, AttendanceStatus::Flagged);
        assert_eq!(r2.status, AttendanceStatus::Flagged);

        act(&t, &c, &r1, "accept", &[("reason", "same day, two sites")])
            .await
            .assert_status(422)
            .assert_see("Site 2")
            .assert_see("13:00")
            .assert_see("17:00");
        assert_eq!(t.reload(&r1).await.status, AttendanceStatus::Flagged);

        // Rejecting needs a reason.
        act(&t, &c, &r2, "reject", &[]).await.assert_status(422);
        act(&t, &c, &r2, "reject", &[("reason", "was at Site 1")])
            .await
            .assert_ok();
        let rejected = t.reload(&r2).await;
        assert_eq!(rejected.status, AttendanceStatus::Rejected);
        let history = t.history_of(&rejected).await;
        let reject = history.iter().find(|h| h.0 == "reject").unwrap();
        assert_eq!(reject.1, Some(c.id));
        assert_eq!(reject.2.as_deref(), Some("was at Site 1"));

        act(&t, &c, &r1, "accept", &[("reason", "was at Site 1")])
            .await
            .assert_ok();
        assert_eq!(t.reload(&r1).await.status, AttendanceStatus::Accepted);
    })
    .await;
}

/// A held flag: the marker B cannot `correct` (with every permission), and a
/// supervisor without `attendance.review` cannot either.
#[renox::test]
async fn att05_the_marker_cannot_correct_past_a_held_flag() {
    let t = boot().await;
    let b = t.user_with("Bashir", "lead").await;
    let s = t.user_with("Samir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = over_cap(&t, a, &site, b.id).await;
        let form = [
            ("check_in", "2026-11-02T07:00"),
            ("check_out", "2026-11-02T15:00"),
            ("reason", "typo"),
        ];
        act(&t, &b, &record, "correct", &form)
            .await
            .assert_status(422)
            .assert_see("HRM.ACL.09");
        act(&t, &s, &record, "correct", &form)
            .await
            .assert_forbidden();
        let same = t.reload(&record).await;
        assert_eq!(same.status, AttendanceStatus::Flagged);
        assert_eq!(same.check_in, at(2, 6, 0));
    })
    .await;
}

/// R1 (Site 1, 07:00-15:00) and R2 (Site 2, 13:00-17:00) flagged `overlap`:
/// reviewer C corrects R2 to the same times and is refused; corrected to
/// 15:00-17:00 with a reason, R2 is `accepted` and R1 is still `flagged`
/// until C accepts it. The original values stay in the history.
#[renox::test]
async fn att05_a_correction_must_end_the_overlap_and_leaves_the_other_record_flagged() {
    let t = boot().await;
    let x = t.user_with("Xavier", "supervisor").await;
    let y = t.user_with("Yasmin", "supervisor").await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(2), async {
        let r1 = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(2, 7, 0), Some(at(2, 15, 0)), x.id),
        )
        .await
        .unwrap();
        let r2 = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(2, 13, 0), Some(at(2, 17, 0)), y.id),
        )
        .await
        .unwrap();

        let unchanged = [
            ("check_in", "2026-11-02T13:00"),
            ("check_out", "2026-11-02T17:00"),
            ("reason", "checked the list"),
        ];
        act(&t, &c, &r2, "correct", &unchanged)
            .await
            .assert_status(422)
            .assert_see("Site 1");
        assert_eq!(t.reload(&r2).await.status, AttendanceStatus::Flagged);
        // No reason, no correction.
        act(
            &t,
            &c,
            &r2,
            "correct",
            &[("check_in", "2026-11-02T15:00"), ("check_out", "2026-11-02T17:00")],
        )
        .await
        .assert_status(422);

        let fixed = [
            ("check_in", "2026-11-02T15:00"),
            ("check_out", "2026-11-02T17:00"),
            ("reason", "left Site 1 at 15:00"),
        ];
        act(&t, &c, &r2, "correct", &fixed).await.assert_ok();
        let r2 = t.reload(&r2).await;
        assert_eq!(r2.status, AttendanceStatus::Accepted);
        assert_eq!(r2.check_in, at(2, 15, 0));
        assert_eq!(r2.check_out, Some(at(2, 17, 0)));
        assert_eq!(t.reload(&r1).await.status, AttendanceStatus::Flagged);

        // The history keeps the reason; the flag list notes the correction.
        let history = renox::state_machine::history(t.db(), AttendanceRecord::TABLE, r2.id)
            .await
            .unwrap();
        let correct = history.iter().find(|h| h.transition == "correct").unwrap();
        assert_eq!(correct.actor_id, Some(c.id));
        assert_eq!(correct.reason.as_deref(), Some("left Site 1 at 15:00"));
        assert_eq!(
            correct.metadata["original"]["check_in"],
            json!(at(2, 13, 0))
        );
        assert_eq!(
            correct.metadata["corrected"]["check_in"],
            json!(at(2, 15, 0))
        );
        assert_eq!(t.flags_of(&r2).await, ["overlap", "manual_correction"]);
        assert!(
            AttendanceFlag::open_of_record(t.db(), r2.id)
                .await
                .unwrap()
                .is_empty()
        );

        act(&t, &c, &r1, "accept", &[("reason", "Site 1 until 15:00")])
            .await
            .assert_ok();
        assert_eq!(t.reload(&r1).await.status, AttendanceStatus::Accepted);
    })
    .await;
}

/// A correction on a record with no held flag needs only
/// `attendance.correct` and a reason, within the window, and runs the checks
/// again: one that creates an overlap flags both records.
#[renox::test]
async fn att05_a_correction_that_creates_an_overlap_flags_both_records() {
    let t = boot().await;
    let s = t.user_with("Samir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    let first = t
        .fixture_by(a, &site1, at(2, 7, 0), Some(at(2, 15, 0)), AttendanceStatus::Accepted, Some(s.id))
        .await;
    let second = t
        .fixture_by(a, &site2, at(2, 16, 0), Some(at(2, 18, 0)), AttendanceStatus::Accepted, Some(s.id))
        .await;
    t.on(nov(2), async {
        // The marker may correct what no flag holds, but not without a reason.
        let form = [
            ("check_in", "2026-11-02T14:00"),
            ("check_out", "2026-11-02T18:00"),
        ];
        act(&t, &s, &second, "correct", &form).await.assert_status(422);
        let with_reason = [
            ("check_in", "2026-11-02T14:00"),
            ("check_out", "2026-11-02T18:00"),
            ("reason", "started at 14:00"),
        ];
        act(&t, &s, &second, "correct", &with_reason).await.assert_ok();
        let second = t.reload(&second).await;
        let first = t.reload(&first).await;
        assert_eq!(second.check_in, at(2, 14, 0));
        assert_eq!(second.status, AttendanceStatus::Flagged);
        assert_eq!(first.status, AttendanceStatus::Flagged);
        assert!(t.flags_of(&second).await.contains(&"overlap".to_owned()));
        assert!(t.flags_of(&first).await.contains(&"overlap".to_owned()));
    })
    .await;
}

/// A correction outside the window, or with the check-out before the
/// check-in, is refused.
#[renox::test]
async fn att05_a_correction_stays_in_the_window_and_in_order() {
    let t = boot().await;
    let s = t.user_with("Samir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    let old = t
        .fixture(a, &site, at(2, 7, 0), Some(at(2, 15, 0)), AttendanceStatus::Accepted)
        .await;
    t.on(nov(6), async {
        act(
            &t,
            &s,
            &old,
            "correct",
            &[
                ("check_in", "2026-11-02T08:00"),
                ("check_out", "2026-11-02T15:00"),
                ("reason", "late"),
            ],
        )
        .await
        .assert_status(422);
        assert_eq!(t.reload(&old).await.check_in, at(2, 7, 0));
    })
    .await;
    let today = t
        .fixture(a, &site, at(6, 7, 0), Some(at(6, 15, 0)), AttendanceStatus::Accepted)
        .await;
    t.on(nov(6), async {
        act(
            &t,
            &s,
            &today,
            "correct",
            &[
                ("check_in", "2026-11-06T15:00"),
                ("check_out", "2026-11-06T07:00"),
                ("reason", "swapped"),
            ],
        )
        .await
        .assert_status(422);
    })
    .await;
}

/// The system accepts a record only when it has no open flag: never one held
/// for review, and not one that is `flagged`.
#[renox::test]
async fn att05_the_system_never_accepts_a_record_with_a_held_flag() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        // Flagged by the cap: the machine does not take it from `flagged`.
        let held = over_cap(&t, a, &site, boss.id).await;
        assert!(review::accept_by_system(t.state(), held.id).await.is_err());
        assert_eq!(t.reload(&held).await.status, AttendanceStatus::Flagged);

        // `recorded` with an open held flag: refused.
        let recorded = t
            .fixture(b, &site, at(2, 7, 0), Some(at(2, 15, 0)), AttendanceStatus::Recorded)
            .await;
        AttendanceFlag::create(
            t.db(),
            AttendanceFlag {
                record_id: recorded.id,
                flag_type: FlagType::PermitInvalid,
                ..AttendanceFlag::default()
            },
        )
        .await
        .unwrap();
        assert!(review::accept_by_system(t.state(), recorded.id).await.is_err());
        assert_eq!(t.reload(&recorded).await.status, AttendanceStatus::Recorded);
    })
    .await;

    // With no flag the system accepts it.
    let c = t.employee(3, &[]);
    let clean = t
        .fixture(c, &site, at(3, 7, 0), Some(at(3, 15, 0)), AttendanceStatus::Recorded)
        .await;
    let accepted = review::accept_by_system(t.state(), clean.id).await.unwrap();
    assert_eq!(accepted.status, AttendanceStatus::Accepted);
    let history = t.history_of(&accepted).await;
    let step = history.iter().find(|h| h.0 == "accept_by_system").unwrap();
    assert_eq!(step.1, None, "run by the system");
}

/// `lock` is the system's, from `accepted` only.
#[renox::test]
async fn att05_an_accepted_record_is_locked_by_the_system_and_then_never_changes() {
    let t = boot().await;
    let s = t.user_with("Samir", "supervisor").await;
    let c = t.user_with("Chitra", "reviewer").await;
    let a = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    let accepted = t
        .fixture(a, &site, at(2, 7, 0), Some(at(2, 15, 0)), AttendanceStatus::Accepted)
        .await;
    let recorded = t
        .fixture(a, &site, at(3, 7, 0), Some(at(3, 15, 0)), AttendanceStatus::Recorded)
        .await;
    let mut tx = t.db().begin().await.unwrap();
    assert!(review::lock_in(&mut tx, recorded.id).await.is_err());
    drop(tx);
    let mut tx = t.db().begin().await.unwrap();
    let outcome = review::lock_in(&mut tx, accepted.id).await.unwrap();
    tx.commit().await.unwrap();
    outcome.emit(t.state()).await.unwrap();
    assert_eq!(t.reload(&accepted).await.status, AttendanceStatus::Locked);

    t.on(nov(2), async {
        act(&t, &c, &accepted, "reject", &[("reason", "no")])
            .await
            .assert_status(422);
        act(
            &t,
            &s,
            &accepted,
            "correct",
            &[
                ("check_in", "2026-11-02T08:00"),
                ("check_out", "2026-11-02T15:00"),
                ("reason", "late"),
            ],
        )
        .await
        .assert_status(422);
        assert_eq!(t.reload(&accepted).await.check_in, at(2, 7, 0));
    })
    .await;
}
