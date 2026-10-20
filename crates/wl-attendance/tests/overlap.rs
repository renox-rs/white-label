//! Overlapping attendance across locations and projects (wl-attendance §4
//! `record`, §5.5 point 4, `HRM.ACL.09` flags, issue #178): supervisor
//! marking and imports are saved and flagged `overlap` through the `flag`
//! transition; the crew list shows the clash before submitting.

mod support;

use wl_attendance::overlap::clashes_for;
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceFlag, AttendanceStatus};

use support::{at, boot, nov, users_of};

/// A has an accepted record at Site 1 for 07:00-15:00; supervisor B marks A
/// at Site 2 for 13:00-17:00 the same day: both are `flagged` `overlap`, two
/// `AttendanceFlagged` events are emitted, the reviewer is told, and each
/// history shows the system `flag` transition with the flag type.
#[renox::test]
async fn att01_marking_over_an_accepted_record_flags_both() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let reviewer = t.user_with("Rita", "reviewer").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    let existing = t
        .fixture(
            a,
            &site1,
            at(2, 7, 0),
            Some(at(2, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    t.on(nov(2), async {
        let new = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(2, 13, 0), Some(at(2, 17, 0)), boss.id),
        )
        .await
        .unwrap();
        let existing = t.reload(&existing).await;
        assert_eq!(new.status, AttendanceStatus::Flagged);
        assert_eq!(existing.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&new).await, ["overlap"]);
        assert_eq!(t.flags_of(&existing).await, ["overlap"]);

        // One AttendanceFlagged per record.
        let events = t.flagged.all();
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|e| e.flag == "overlap" && e.employee == a)
        );
        let mut ids: Vec<i64> = events.iter().map(|e| e.record.0).collect();
        ids.sort_unstable();
        assert_eq!(ids, [existing.id, new.id]);
        // The record is saved, and AttendanceRecorded names the flag.
        let recorded = t.recorded.all();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].flags, ["overlap"]);

        // The reviewer is told, once per record; the supervisor knows too.
        let told = t.notices.flagged("overlap");
        assert_eq!(told.len(), 2);
        assert!(told.iter().all(|r| users_of(r).contains(&reviewer.id)));
        let about_new = told
            .iter()
            .find(|r| r.subject == new.record_id().subject())
            .expect("the new record is announced");
        assert!(users_of(about_new).contains(&boss.id));

        // The state was moved by the transition, not written.
        for record in [&new, &existing] {
            let flags: Vec<_> = t
                .history_of(record)
                .await
                .into_iter()
                .filter(|h| h.0 == "flag")
                .collect();
            assert_eq!(flags.len(), 1, "{flags:?}");
            assert_eq!(flags[0].1, None, "run by the system");
            assert_eq!(flags[0].2.as_deref(), Some("overlap"));
            assert_eq!(flags[0].3.as_deref(), Some("overlap"));
        }
        // The note names the other record and its place.
        let notes = AttendanceFlag::of_record(t.db(), new.id).await.unwrap();
        assert!(notes[0].note.contains("Site 1"), "{}", notes[0].note);
        assert!(notes[0].note.contains("07:00"), "{}", notes[0].note);
    })
    .await;
}

/// Before submitting, the crew list of Site 2 shows A as already marked at
/// Site 1 for 07:00-15:00; with the hours of the marking, only a real clash.
#[renox::test]
async fn att01_the_crew_list_shows_a_worker_marked_elsewhere() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let b = t.employee(2, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.fixture(
        a,
        &site1,
        at(2, 7, 0),
        Some(at(2, 15, 0)),
        AttendanceStatus::Accepted,
    )
    .await;
    t.on(nov(2), async {
        let all = clashes_for(t.state(), site2.id, nov(2), &[a, b], None)
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].employee, a);
        assert_eq!(all[0].location, "Site 1");
        assert_eq!(all[0].hours, "07:00\u{2013}15:00");

        let clash = clashes_for(
            t.state(),
            site2.id,
            nov(2),
            &[a],
            Some((at(2, 13, 0), at(2, 17, 0))),
        )
        .await
        .unwrap();
        assert_eq!(clash.len(), 1);
        let later = clashes_for(
            t.state(),
            site2.id,
            nov(2),
            &[a],
            Some((at(2, 15, 0), at(2, 17, 0))),
        )
        .await
        .unwrap();
        assert!(later.is_empty(), "touching hours are no clash");
        // Her own location is not "elsewhere".
        let own = clashes_for(t.state(), site1.id, nov(2), &[a], None)
            .await
            .unwrap();
        assert!(own.is_empty());
    })
    .await;
}

/// Spans that touch are not an overlap: no flag, no event.
#[renox::test]
async fn att01_touching_spans_do_not_overlap() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    let morning = t
        .fixture(
            a,
            &site1,
            at(2, 7, 0),
            Some(at(2, 12, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    t.on(nov(2), async {
        let afternoon = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(2, 12, 0), Some(at(2, 16, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(afternoon.status, AttendanceStatus::Recorded);
        assert!(t.flags_of(&afternoon).await.is_empty());
        assert_eq!(t.reload(&morning).await.status, AttendanceStatus::Accepted);
        assert!(t.flagged.all().is_empty());
        assert!(t.notices.all().is_empty());
    })
    .await;
}

/// A `locked` record cannot change: only the new record is flagged, and its
/// note names the locked one.
#[renox::test]
async fn att01_a_locked_record_is_not_touched() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    let locked = t
        .fixture(
            a,
            &site1,
            at(2, 7, 0),
            Some(at(2, 15, 0)),
            AttendanceStatus::Locked,
        )
        .await;
    t.on(nov(2), async {
        let new = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(2, 13, 0), Some(at(2, 17, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(new.status, AttendanceStatus::Flagged);
        let after = t.reload(&locked).await;
        assert_eq!(after.status, AttendanceStatus::Locked);
        assert!(t.flags_of(&after).await.is_empty());
        let notes = AttendanceFlag::of_record(t.db(), new.id).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0]
                .note
                .contains(&format!("locked record {}", locked.id)),
            "{}",
            notes[0].note
        );
        assert_eq!(t.flagged.all().len(), 1);
    })
    .await;
}

/// A `rejected` record does not count.
#[renox::test]
async fn att01_a_rejected_record_does_not_count() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    let rejected = t
        .fixture(
            a,
            &site1,
            at(2, 7, 0),
            Some(at(2, 15, 0)),
            AttendanceStatus::Rejected,
        )
        .await;
    t.on(nov(2), async {
        let new = service::record(
            t.state(),
            NewRecord::supervisor(a, site2.id, at(2, 13, 0), Some(at(2, 17, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(new.status, AttendanceStatus::Recorded);
        assert_eq!(t.reload(&rejected).await.status, AttendanceStatus::Rejected);
        assert!(t.flagged.all().is_empty());
        let shown = clashes_for(t.state(), site2.id, nov(2), &[a], None)
            .await
            .unwrap();
        assert!(shown.is_empty(), "a rejected record is not a clash");
    })
    .await;
}

/// An import that overlaps is saved and flagged like a supervisor's marking;
/// the day before and the day after are looked at too (a span can cross
/// midnight).
#[renox::test]
async fn att01_an_import_that_overlaps_is_saved_and_flagged() {
    let t = boot().await;
    let a = t.employee(1, &[]);
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    // A night shift from 22:00 on the 1st to 06:00 on the 2nd.
    let night = t
        .fixture(
            a,
            &site1,
            at(1, 22, 0),
            Some(at(2, 6, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    t.on(nov(2), async {
        let imported = service::record(
            t.state(),
            NewRecord::import(a, site2.id, at(2, 5, 0), Some(at(2, 9, 0))),
        )
        .await
        .unwrap();
        assert_eq!(imported.recorded_by, None);
        assert_eq!(imported.status, AttendanceStatus::Flagged);
        assert_eq!(t.reload(&night).await.status, AttendanceStatus::Flagged);
    })
    .await;
}

/// Two supervisors submit for the same worker at the same moment: both
/// records exist and both are `flagged` `overlap`, run 20 times; never one
/// unflagged record.
#[renox::test]
async fn att01_two_parallel_markings_flag_both_records() {
    let t = boot().await;
    let b = t.user_with("Bashir", "supervisor").await;
    let c = t.user_with("Chitra", "supervisor").await;
    let site1 = t.location("Site 1").await;
    let site2 = t.location("Site 2").await;
    t.on(nov(2), async {
        for n in 0..20 {
            let a = t.employee(100 + n, &[]);
            let (first, second) = renox::tokio::join!(
                service::record(
                    t.state(),
                    NewRecord::supervisor(a, site1.id, at(2, 7, 0), Some(at(2, 15, 0)), b.id),
                ),
                service::record(
                    t.state(),
                    NewRecord::supervisor(a, site2.id, at(2, 13, 0), Some(at(2, 17, 0)), c.id),
                ),
            );
            first.unwrap();
            second.unwrap();
            let records = t.records_of(a).await;
            assert_eq!(records.len(), 2, "run {n}");
            for record in &records {
                assert_eq!(record.status, AttendanceStatus::Flagged, "run {n}");
                assert_eq!(t.flags_of(record).await, ["overlap"], "run {n}");
            }
            let events = t
                .flagged
                .all()
                .into_iter()
                .filter(|e| e.employee == a)
                .count();
            assert_eq!(events, 2, "run {n}: one event per record");
        }
    })
    .await;
}
