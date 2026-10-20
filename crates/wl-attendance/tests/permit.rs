//! `HRM.RUL.14`, work permit, residence visa and qualifications (wl-attendance
//! §5.5 point 3, issue #181): a record for a day on which one of them is not
//! valid is flagged `permit_invalid` the same day; HR and the supervisor are
//! told; the record is not refused; while it is flagged it proposes no hours,
//! and after `accept` they are non-billable.

mod support;

use wl_attendance::proposal::{self, Proposal};
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceFlag, AttendanceStatus, WlAttendance, WorkLocation};

use support::{at, boot, boot_with, nov, users_of};

/// A's work permit expired on 1 November; 2 November: marked present. The
/// record is saved, flagged `permit_invalid`, and HR and the supervisor are
/// told.
#[renox::test]
async fn att04_an_expired_work_permit_flags_the_record_and_tells_hr_and_the_supervisor() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let hana = t.user_with("Hana", "hr").await;
    let a = t.employee(1, &[]);
    t.docs
        .accepted_until(&a.subject(), "work_permit", 11, Some(nov(1)));
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        // Not refused: the worker did turn up.
        assert_eq!(record.status, AttendanceStatus::Flagged);
        assert_eq!(record.check_out, Some(at(2, 15, 0)));
        assert_eq!(t.flags_of(&record).await, ["permit_invalid"]);
        let note = AttendanceFlag::of_record(t.db(), record.id)
            .await
            .unwrap()
            .remove(0)
            .note;
        assert!(
            note.contains("work_permit") && !note.contains("residence_visa"),
            "{note}"
        );

        let history = t.history_of(&record).await;
        let flags: Vec<_> = history.iter().filter(|h| h.0 == "flag").collect();
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].1, None, "by the system");
        assert_eq!(flags[0].3.as_deref(), Some("permit_invalid"));

        let told = t.notices.flagged("permit_invalid");
        assert_eq!(told.len(), 1);
        let users = users_of(&told[0]);
        assert!(users.contains(&hana.id), "HR");
        assert!(users.contains(&boss.id), "the supervisor");
        assert_eq!(t.flagged.all().len(), 1);
    })
    .await;
}

/// A residence visa that expired, or one that is missing or waits for review,
/// flags the record too; valid papers do not.
#[renox::test]
async fn att04_the_residence_visa_counts_and_a_pending_renewal_is_no() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let fine = t.employee(1, &[]);
    let expired = t.employee(2, &[]);
    let pending = t.employee(3, &[]);
    let site = t.location("Site 1").await;
    t.docs
        .accepted_until(&expired.subject(), "residence_visa", 21, Some(nov(1)));
    t.docs
        .accepted_until(&pending.subject(), "residence_visa", 31, Some(nov(1)));
    t.docs
        .pending_review(&pending.subject(), "residence_visa", 32);
    t.on(nov(2), async {
        let mark = |employee| {
            NewRecord::supervisor(employee, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id)
        };
        let ok = service::record(t.state(), mark(fine)).await.unwrap();
        assert_eq!(ok.status, AttendanceStatus::Recorded);
        let bad = service::record(t.state(), mark(expired)).await.unwrap();
        assert_eq!(t.flags_of(&bad).await, ["permit_invalid"]);
        let waiting = service::record(t.state(), mark(pending)).await.unwrap();
        assert_eq!(t.flags_of(&waiting).await, ["permit_invalid"]);
    })
    .await;
}

/// A context that requires a SIRA licence: an expired licence flags the
/// record, a valid one does not. The role and the scope of the context are
/// passed for a role-bound licence.
#[renox::test]
async fn att04_a_qualification_the_context_requires_is_checked() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let guard = t.employee(1, &[]);
    let lapsed = t.employee(2, &[]);
    let bound = t.employee(3, &[]);
    let wrong_site = t.employee(4, &[]);
    let tower = support::project(7);
    let site = t
        .location_of(
            WorkLocation::named("Tower")
                .in_scope(&tower)
                .requiring("sira_licence")
                .for_role("guard"),
        )
        .await;
    t.quals.grant(guard, "sira_licence", Some(nov(30)));
    t.quals.grant(lapsed, "sira_licence", Some(nov(1)));
    t.quals.bound(
        bound,
        "sira_licence",
        &["guard"],
        std::slice::from_ref(&tower),
        None,
    );
    t.quals.bound(
        wrong_site,
        "sira_licence",
        &["guard"],
        &[support::project(8)],
        None,
    );
    t.on(nov(2), async {
        let mark = |employee| {
            NewRecord::supervisor(employee, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id)
        };
        for employee in [guard, bound] {
            let record = service::record(t.state(), mark(employee)).await.unwrap();
            assert_eq!(record.status, AttendanceStatus::Recorded, "{employee}");
        }
        for employee in [lapsed, wrong_site] {
            let record = service::record(t.state(), mark(employee)).await.unwrap();
            assert_eq!(t.flags_of(&record).await, ["permit_invalid"], "{employee}");
        }
        let told = t.notices.flagged("permit_invalid");
        assert_eq!(told.len(), 2);
        assert!(
            told[0].data["note"]
                .as_str()
                .unwrap()
                .contains("sira_licence")
        );
    })
    .await;
}

/// A late entry on 3 November for 2 November is checked against 2 November:
/// the permit expired on 2 November is valid then; marked for 3 November it is
/// not.
#[renox::test]
async fn att04_the_permit_is_checked_on_the_attendance_date() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    t.docs
        .accepted_until(&a.subject(), "work_permit", 11, Some(nov(2)));
    let site = t.location("Site 1").await;
    t.on(nov(3), async {
        let late = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(late.date, nov(2));
        assert_eq!(late.status, AttendanceStatus::Recorded);
        assert!(t.flags_of(&late).await.is_empty());

        let today = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, at(3, 7, 0), Some(at(3, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(t.flags_of(&today).await, ["permit_invalid"]);
    })
    .await;
}

/// The flag stays when the document is renewed later the same day: HR clears
/// it by review.
#[renox::test]
async fn att04_a_renewal_later_the_same_day_does_not_clear_the_flag() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let a = t.employee(1, &[]);
    t.docs
        .accepted_until(&a.subject(), "work_permit", 11, Some(nov(1)));
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, at(2, 7, 0), None, boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
        t.docs
            .accepted_until(&a.subject(), "work_permit", 12, Some(nov(30)));
        let closed = service::check_out(t.state(), record.id, at(2, 15, 0))
            .await
            .unwrap();
        assert_eq!(closed.status, AttendanceStatus::Flagged);
        assert_eq!(t.flags_of(&closed).await, ["permit_invalid"]);
    })
    .await;
}

/// With the setting off, or the kind left out, no flag is set.
#[renox::test]
async fn att04_the_setting_and_the_kinds_of_the_client_are_followed() {
    let off = boot_with(WlAttendance::new().permit_check(false), "").await;
    let boss = off.user_with("Bashir", "supervisor").await;
    let a = off.employee(1, &[]);
    off.docs
        .accepted_until(&a.subject(), "work_permit", 11, Some(nov(1)));
    let site = off.location("Site 1").await;
    let requiring = off
        .location_of(WorkLocation::named("Tower").requiring("sira_licence"))
        .await;
    off.on(nov(2), async {
        let record = service::record(
            off.state(),
            NewRecord::supervisor(a, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Recorded);
        // Not even a required qualification is checked.
        let tower = service::record(
            off.state(),
            NewRecord::supervisor(a, requiring.id, at(2, 16, 0), Some(at(2, 18, 0)), boss.id),
        )
        .await
        .unwrap();
        assert!(off.flags_of(&tower).await.is_empty());
    })
    .await;

    // A client that checks the work permit only: a missing visa is no flag.
    let permit_only = boot_with(WlAttendance::new().permit_kinds(&["work_permit"]), "").await;
    let boss = permit_only.user_with("Bashir", "supervisor").await;
    let b = permit_only.employee(2, &[]);
    permit_only
        .docs
        .accepted_until(&b.subject(), "residence_visa", 21, Some(nov(1)));
    let site = permit_only.location("Site 1").await;
    permit_only
        .on(nov(2), async {
            let record = service::record(
                permit_only.state(),
                NewRecord::supervisor(b, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
            )
            .await
            .unwrap();
            assert_eq!(record.status, AttendanceStatus::Recorded);
        })
        .await;
}

/// While the record is `flagged` it proposes no hours and the timesheet's
/// `submit` is refused with a message that names it; once a reviewer accepts
/// it (`HRM.ACL.09`), its hours are proposed non-billable and count (a normal
/// record's are billable) and `submit` is no longer held.
#[renox::test]
async fn att04_a_flagged_record_proposes_nothing_and_an_accepted_one_non_billable() {
    let t = boot().await;
    let boss = t.user_with("Bashir", "supervisor").await;
    let hr = t.user_with("Hana", "reviewer").await;
    let a = t.employee(1, &[]);
    t.docs
        .accepted_until(&a.subject(), "work_permit", 11, Some(nov(1)));
    let site = t.location("Site 1").await;
    t.on(nov(2), async {
        let record = service::record(
            t.state(),
            NewRecord::supervisor(a, site.id, at(2, 7, 0), Some(at(2, 15, 0)), boss.id),
        )
        .await
        .unwrap();
        assert_eq!(record.status, AttendanceStatus::Flagged);
        assert_eq!(
            proposal::proposal_of(t.state(), record.id).await.unwrap(),
            None
        );
        let blocked = proposal::submit_blocker(t.state(), a, nov(2), nov(8))
            .await
            .unwrap()
            .expect("submit is held");
        assert!(
            blocked.contains("Site 1") && blocked.contains("2026-11-02"),
            "{blocked}"
        );
        assert!(
            blocked.contains(&format!("record {}", record.id)),
            "{blocked}"
        );
        assert!(blocked.contains("permit_invalid"), "{blocked}");
        // Another period is not held by it.
        assert_eq!(
            proposal::submit_blocker(t.state(), a, nov(9), nov(15))
                .await
                .unwrap(),
            None
        );

        // HR reviews it: the real `accept`, with a reason.
        t.app.acting_as(&hr);
        t.app
            .htmx()
            .post(
                &format!("/attendance/records/{}/accept", record.id),
                &[("reason", "renewal in process, HR verified")],
            )
            .await
            .assert_ok();
        let accepted = t.reload(&record).await;
        assert_eq!(accepted.status, AttendanceStatus::Accepted);
        assert_eq!(
            proposal::proposal_of(t.state(), record.id).await.unwrap(),
            Some(Proposal {
                minutes: 480,
                billable: false
            })
        );
        assert_eq!(
            proposal::submit_blocker(t.state(), a, nov(2), nov(8))
                .await
                .unwrap(),
            None
        );
    })
    .await;

    let normal = t
        .fixture(
            a,
            &site,
            at(4, 7, 0),
            Some(at(4, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    assert_eq!(
        proposal::proposal_of(t.state(), normal.id).await.unwrap(),
        Some(Proposal {
            minutes: 480,
            billable: true
        })
    );
}

/// `DOC.RUL.02`, the crews affected: the look-ahead of a work permit that
/// expires on 2027-05-01 lists the employee of the crew from 2027-05-02, under
/// their site; an employee who is gone, has no site, or another subject
/// is not listed.
#[renox::test]
async fn att04_the_crews_affected_by_an_expiring_permit_are_listed() {
    use wl_kernel::documents::{BlockEntry, BlockMode, OverrideMode, future_work};

    let t = boot().await;
    let site = support::project(1);
    let a = t.employee(1, &[]);
    let gone = t.employee(2, &[]);
    let loose = t.employee(3, &[]);
    for employee in [a, gone] {
        t.staff
            .change(employee, |profile| profile.scopes = vec![site.clone()]);
    }
    t.staff.change(gone, |profile| profile.listed = false);
    let entry = BlockEntry {
        kind: "work_permit".into(),
        guarded: "attendance_record.accept".into(),
        mode: BlockMode::Flag,
        override_mode: OverrideMode::Never,
        condition: None,
        locked: false,
    };
    let provider = future_work("attendance_record.accept").expect("the module registers it");
    let from = nov(2);
    let planned = |subject: wl_kernel::Subject| {
        let (provider, entry, state) = (provider.clone(), entry.clone(), t.state());
        async move {
            provider
                .planned_from(state, &entry, &subject, from)
                .await
                .unwrap()
        }
    };

    let found = planned(a.subject()).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].record, a.subject());
    assert_eq!(found[0].date, from);
    assert!(found[0].label.contains("Employee 1"), "{}", found[0].label);
    assert_eq!(
        found[0].scope,
        Some(renox::auth::permissions::Scope::new(
            &site.subject_type,
            site.subject_id
        ))
    );
    assert!(planned(gone.subject()).await.is_empty());
    assert!(planned(loose.subject()).await.is_empty());
    assert!(planned(support::project(9)).await.is_empty());
}
