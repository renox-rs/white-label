//! `HRM.RUL.13`, the overtime cap (wl-attendance §5.5 point 1, issue #179):
//! the day against the calendar's normal hours plus 2 h, and the rolling 21
//! days against 144 h; the flag does not block.

mod support;

use wl_attendance::overtime::{self, DayCheck};
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceStatus, WlAttendance};
use wl_kernel::calendar::calendar;

use support::{at, boot, boot_with, nov};

/// Ramadan, in November for the tests: 9 to 13 November, two hours shorter.
const RAMADAN: &str = r#"
[calendar]
normal_hours = 8

[[calendar.reduced_hours]]
from = "2026-11-09"
to = "2026-11-13"
hours_less = 2
"#;

/// A has 07:00-13:00 at Site 1 and 13:30-18:00 at Site 2 on Tuesday
/// 3 November, 10.5 h with 8 h of normal hours: both records are flagged
/// `overtime_cap`, the supervisor is told, and the records are saved.
#[renox::test]
async fn att02_ten_and_a_half_hours_flag_both_records_and_tell_the_supervisor() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let reviewer = t.user_with("Rita", "reviewer").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(3), async {
        let first = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 7, 0), Some(at(3, 13, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(first.status, AttendanceStatus::Recorded, "6 h alone is fine");
        assert!(t.flagged.all().is_empty());

        let second = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(3, 13, 30), Some(at(3, 18, 0)), boss.id),
        )
        .await
        .unwrap();
        let first = t.reload(&first).await;
        for record in [&first, &second] {
            assert_eq!(record.status, AttendanceStatus::Flagged);
            assert_eq!(t.flags_of(record).await, ["overtime_cap"]);
            let flag: Vec<_> = t
                .history_of(record)
                .await
                .into_iter()
                .filter(|h| h.0 == "flag")
                .collect();
            assert_eq!(flag.len(), 1);
            assert_eq!(flag[0].3.as_deref(), Some("overtime_cap"));
        }
        assert_eq!(t.flagged.all().len(), 2);
        // The supervisor and the reviewer are told, once per record.
        let told = t.notices.flagged("overtime_cap");
        assert_eq!(told.len(), 2);
        assert!(told.iter().all(|r| {
            let users = support::users_of(r);
            users.contains(&boss.id) && users.contains(&reviewer.id)
        }));
        assert!(told[0].data["note"].as_str().unwrap().contains("10.5"));
    })
    .await;
}

/// Ten hours in total, with 8 normal hours plus 2 allowed: no flag.
#[renox::test]
async fn att02_ten_hours_in_total_are_within_the_cap() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(3), async {
        service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 7, 0), Some(at(3, 13, 0)), boss.id),
        )
        .await
        .unwrap();
        service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(3, 13, 0), Some(at(3, 17, 0)), boss.id),
        )
        .await
        .unwrap();
        for record in t.records_of(a).await {
            assert_eq!(record.status, AttendanceStatus::Recorded);
            assert!(t.flags_of(&record).await.is_empty());
        }
        assert!(t.flagged.all().is_empty());
        assert!(t.notices.all().is_empty());
    })
    .await;
}

/// In Ramadan the normal hours are 6: 8.5 h are flagged where the same hours
/// on an ordinary day are not.
#[renox::test]
async fn att02_ramadan_makes_the_normal_hours_shorter() {
    let t = boot_with(WlAttendance::new(), RAMADAN).await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(3), async {
        let ordinary = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 7, 0), Some(at(3, 15, 30)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(ordinary.status, AttendanceStatus::Recorded);
    })
    .await;
    t.on(nov(10), async {
        let fasting = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(10, 7, 0), Some(at(10, 15, 30)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(fasting.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&fasting).await, ["overtime_cap"]);
    })
    .await;
}

/// The cap is the setting's, not 2 h: with 1 h a day, 9.5 h are flagged.
#[renox::test]
async fn att02_the_cap_follows_the_setting() {
    let t = boot_with(WlAttendance::new().overtime_cap(1, 144), "").await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(3), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 7, 0), Some(at(3, 16, 30)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// A rest day has no normal hours: 2 h are within the cap, more are not.
#[renox::test]
async fn att02_a_rest_day_allows_two_hours() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site1 = t.location("Site 1").await;
    // Saturday 7 November.
    t.on(nov(7), async {
        let two = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(7, 7, 0), Some(at(7, 9, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(two.status, AttendanceStatus::Recorded);
        let three = service::record(
            t.state(),
            NewRecord::supervisor(b, site1.id, at(7, 7, 0), Some(at(7, 10, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(three.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// Overtime of 20 days at 7 h and a 10 h day on the 30th: 145 h in the window
/// are over the cap although the day's own overtime (2 h) is within it; with
/// 142 h before, the window holds exactly 144 h and nothing is flagged.
#[renox::test]
async fn att02_overtime_above_144_hours_in_21_days_is_flagged() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let over = t.employee(1, &[]);
    let exact = t.employee(2, &[]);
    let site1 = t.location("Site 1").await;
    let calendar = calendar(t.state());
    // 10 to 29 November: 7 h above the normal hours each day, but the 29th,
    // where `over` has 10 h and `exact` 9 h (a Sunday: all of it overtime).
    for employee in [over, exact] {
        for day in 10..=29 {
            let normal = calendar.normal_minutes_on(nov(day)) / 60;
            let extra = match (employee == over, day) {
                (true, 29) => 10,
                (false, 29) => 9,
                _ => 7,
            };
            let hours = u32::try_from(normal).unwrap() + extra;
            t.fixture(
                employee,
                &site1,
                at(day, 7, 0),
                Some(at(day, 7 + hours, 0)),
                AttendanceStatus::Accepted,
            )
            .await;
        }
    }
    t.on(nov(30), async {
        // Monday: 10 h with 8 normal hours, 2 h of overtime within the daily cap.
        let flagged = service::record(
            t.state(),
            NewRecord::supervisor(over, site1.id, at(30, 7, 0), Some(at(30, 17, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(flagged.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&flagged).await, ["overtime_cap"]);
        let flags = wl_attendance::AttendanceFlag::of_record(t.db(), flagged.id)
            .await
            .unwrap();
        assert!(flags[0].note.contains("145.0"), "{}", flags[0].note);

        let fine = service::record(
            t.state(),
            NewRecord::supervisor(exact, site1.id, at(30, 7, 0), Some(at(30, 17, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(fine.status, AttendanceStatus::Recorded, "144 h is not above");
        assert!(t.flags_of(&fine).await.is_empty());
        // Only the record of the 30th was flagged, not the days before.
        assert_eq!(t.flagged.all().len(), 1);
    })
    .await;
}

/// A timesheet entry that raises the day to 11 h flags the day's open records;
/// saving again does not add a second flag.
#[renox::test]
async fn att02_a_timesheet_entry_flags_the_days_records_once() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(3), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 7, 0), Some(at(3, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Recorded);

        // Three hours entered by hand raise the day to 11 h.
        t.entries.set(a, nov(3), 180);
        let found = overtime::on_entry_saved(t.state(), a, nov(3)).await.unwrap();
        assert!(matches!(found, DayCheck::Over { flaggable: true, .. }));
        let after = t.reload(&record).await;
        assert_eq!(after.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&after).await, ["overtime_cap"]);
        assert_eq!(t.notices.flagged("overtime_cap").len(), 1);
        assert!(support::users_of(&t.notices.flagged("overtime_cap")[0]).contains(&boss.id));

        // Saved again, and the record saved again: still one flag.
        overtime::on_entry_saved(t.state(), a, nov(3)).await.unwrap();
        assert_eq!(t.flags_of(&after).await, ["overtime_cap"]);
        assert_eq!(t.flagged.all().len(), 1);
        assert_eq!(t.notices.flagged("overtime_cap").len(), 1);

        // The entries within the cap flag nothing.
        t.entries.set(a, nov(3), 0);
        let within = overtime::on_entry_saved(t.state(), a, nov(3)).await.unwrap();
        assert!(matches!(within, DayCheck::Within));
    })
    .await;
}

/// A `locked` record cannot be flagged: the others of the day are; when all
/// are locked the supervisor is told without a flag.
#[renox::test]
async fn att02_locked_records_are_left_alone() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    let locked = t
        .fixture_by(
            a,
            &site1,
            at(3, 7, 0),
            Some(at(3, 15, 0)),
            AttendanceStatus::Locked,
            Some(boss.id),
        )
        .await;
    let only_locked = t
        .fixture_by(
            b,
            &site1,
            at(3, 7, 0),
            Some(at(3, 15, 0)),
            AttendanceStatus::Locked,
            Some(boss.id),
        )
        .await;
    t.on(nov(3), async {
        // Another record the same day takes the day over the cap: only it is flagged.
        let new = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(3, 15, 30), Some(at(3, 18, 30)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(new.status, AttendanceStatus::Flagged);
        let after = t.reload(&locked).await;
        assert_eq!(after.status, AttendanceStatus::Locked);
        assert!(t.flags_of(&after).await.is_empty());

        // Every record of b's day is locked: nothing to flag, the supervisor is told.
        t.entries.set(b, nov(3), 180);
        let found = overtime::on_entry_saved(t.state(), b, nov(3)).await.unwrap();
        assert!(matches!(found, DayCheck::Over { flaggable: false, .. }));
        assert_eq!(t.reload(&only_locked).await.status, AttendanceStatus::Locked);
        assert!(t.flags_of(&only_locked).await.is_empty());
        let told = t.notices.of_type("attendance.over_cap");
        assert_eq!(told.len(), 1);
        assert_eq!(support::users_of(&told[0]), [boss.id]);
    })
    .await;
}

/// Hours above the cap are saved untouched: the flag does not block or trim.
#[renox::test]
async fn att02_the_flag_does_not_trim_the_hours() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    t.on(nov(3), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site1.id, at(3, 6, 0), Some(at(3, 22, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
        assert_eq!(record.check_in, at(3, 6, 0));
        assert_eq!(record.check_out, Some(at(3, 22, 0)));
    })
    .await;
}
