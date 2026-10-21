//! BL-SCH-07 (#174): `SCH.RUL.05` flags the bookings whose resource will not
//! hold a required licence at their start, with the real `wl-hr`
//! (`qualification_valid` through the document desk of the support).
//!
//! The expiry events read the real clock, so the days are relative to today
//! (`day`): the licence lapses on day 19, as on 20 November in the spec
//! scenario of a run on 1 November, with bookings on days 17 and 24 and one
//! beyond the 30 day look-ahead on day 44.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::documents::{
    AffectedGroup, BlockEntry, BlockMode, BlockedGroup, DocumentAccepted, DocumentExpired,
    DocumentExpiring, OverrideMode, PlannedRecord, block_subjects, dubai_date, future_work,
};
use wl_kernel::scheduling::ConflictCause;
use wl_kernel::tickets::Assignee;
use wl_kernel::workforce::{AffectedWork, EmployeeId, QualificationExpired, QualificationExpiring};
use wl_kernel::{DocumentId, Subject};
use wl_scheduling::eligibility::{self, LookAhead};
use wl_scheduling::{AvailabilityBlock, Booking, Conflict, Resource, ServiceType, conflict};

use support::{Booked, LICENCE, boot_hr, dubai};

/// The licence document.
const DOC: i64 = 7;

fn day(days: i64) -> NaiveDate {
    dubai_date(renox::db::now()) + TimeDelta::days(days)
}

fn at(day: NaiveDate, hour: u32) -> DateTime {
    dubai(&day.to_string(), hour, 0)
}

/// Dr A, a licence valid for ever, and three bookings of the licensed
/// service: days 17, 24 and 44 at 10:00.
struct Scene {
    t: Booked,
    doctor: Resource,
    service: ServiceType,
    subject: Subject,
    early: Booking,
    late: Booking,
    beyond: Booking,
}

async fn scene() -> Scene {
    let t = boot_hr().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.licensed_service(&[&doctor]).await;
    let subject = EmployeeId(doctor.employee_id.expect("an employee")).subject();
    let early = t.booking(&service, &[&doctor], at(day(17), 10)).await;
    let late = t.booking(&service, &[&doctor], at(day(24), 10)).await;
    let beyond = t.booking(&service, &[&doctor], at(day(44), 10)).await;
    Scene {
        t,
        doctor,
        service,
        subject,
        early,
        late,
        beyond,
    }
}

impl Scene {
    /// The licence is valid through `until` from now on.
    fn licence_until(&self, document: i64, until: NaiveDate) {
        self.t
            .docs
            .accepted_until(&self.subject, LICENCE, document, Some(until));
    }

    async fn reload(&self, booking: &Booking) -> Booking {
        self.t.reload(booking.id).await
    }

    fn planned(booking: &Booking) -> PlannedRecord {
        PlannedRecord {
            record: booking.subject(),
            date: dubai_date(booking.starts_at),
            label: booking.number.clone(),
            scope: None,
        }
    }

    async fn rows(&self, booking: &Booking) -> Vec<Conflict> {
        Conflict::query()
            .where_(Conflict::BOOKING_ID.eq(booking.id))
            .order_by(Conflict::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

/// Done when 1: the licence lapses on day 19; the run before it flags only
/// the booking on day 24 (cause `eligibility`) and the front desk gets a task.
#[renox::test]
async fn the_nightly_run_flags_only_the_booking_after_the_expiry() {
    let s = scene().await;
    let desk = s.t.front_desk().await;
    s.licence_until(DOC, day(19));

    let done = eligibility::look_ahead(s.t.state(), at(day(0), 2))
        .await
        .unwrap();

    assert_eq!(
        done,
        LookAhead {
            flagged: 1,
            cleared: 0
        }
    );
    assert!(!s.reload(&s.early).await.conflict_eligibility);
    let late = s.reload(&s.late).await;
    assert!(late.conflict_eligibility);
    assert!(!late.conflict_block);
    assert!(
        !s.reload(&s.beyond).await.conflict_eligibility,
        "beyond the look-ahead"
    );
    let rows = s.rows(&late).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cause, "eligibility");
    assert_eq!(rows[0].item.as_deref(), Some(LICENCE));
    assert_eq!(rows[0].resource_id, s.doctor.id);
    assert!(rows[0].is_open());
    // One `BookingConflict` with the item and the booking.
    let events = s.t.events.conflicts.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cause, ConflictCause::Eligibility);
    assert_eq!(events[0].item.as_deref(), Some(LICENCE));
    assert_eq!(events[0].resource, s.doctor.id);
    assert_eq!(events[0].bookings, vec![late.booking_id()]);
    assert_eq!(events[0].earliest_start, late.starts_at);
    // The front desk's task to reassign or reschedule it.
    let tasks = s.t.tasks.with_key("scheduling.eligibility:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(format!("scheduling.eligibility:{}:{LICENCE}", late.id).as_str())
    );
    assert_eq!(tasks[0].due_at, Some(late.starts_at));
    assert_eq!(tasks[0].assignee, Some(Assignee::user(desk.id)));
    assert!(tasks[0].title.contains(&late.number));
    // The party is told nothing yet, and never the reason.
    assert!(
        s.t.messages
            .sent()
            .iter()
            .all(|m| !m.data.to_string().contains("licence"))
    );
    // The booking itself is unchanged: nothing is cancelled or moved.
    assert_eq!(late.starts_at, s.late.starts_at);
    assert_eq!(late.status, s.late.status);
    assert_eq!(
        late.resource_ids(s.t.db()).await.unwrap(),
        vec![s.doctor.id]
    );
}

/// Done when 2: a booking beyond the 30 days is not flagged now; the run that
/// finds it inside the window flags it.
#[renox::test]
async fn a_booking_beyond_the_look_ahead_is_flagged_when_it_enters_the_window() {
    let s = scene().await;
    s.licence_until(DOC, day(19));

    eligibility::look_ahead(s.t.state(), at(day(0), 2))
        .await
        .unwrap();
    assert!(!s.reload(&s.beyond).await.conflict_eligibility);

    // Day 16 + 30 days reaches day 46.
    let done = eligibility::look_ahead(s.t.state(), at(day(16), 2))
        .await
        .unwrap();
    assert_eq!(done.flagged, 1, "only the new booking");
    assert!(s.reload(&s.beyond).await.conflict_eligibility);
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 2);
}

/// Done when 1, through the job: the command the scheduler runs at 02:00.
#[renox::test]
async fn the_scheduled_command_runs_the_look_ahead() {
    let s = scene().await;
    s.licence_until(DOC, day(19));

    s.t.app
        .kernel()
        .call(eligibility::TASK, Vec::<String>::new())
        .await
        .expect("the command runs");

    assert!(s.reload(&s.late).await.conflict_eligibility);
    assert!(!s.reload(&s.early).await.conflict_eligibility);
}

/// Done when 1 and 2, through the registered entry: the task is the 02:00
/// Asia/Dubai one, and run with the clock moved to day 16 it reaches the
/// booking of day 44 that was beyond the look-ahead on day 0, besides the one
/// of day 24; the booking before the expiry stays.
#[renox::test]
async fn the_registered_nightly_task_runs_the_look_ahead_at_the_moved_clock() {
    let s = scene().await;
    s.licence_until(DOC, day(19));
    assert_eq!(eligibility::TASK_AT, "02:00");
    assert_eq!(eligibility::TASK_ZONE, "Asia/Dubai");

    s.t.app
        .travel((at(day(16), 2) - renox::db::now()).to_std().expect("ahead"));
    s.t.app
        .run_scheduled(eligibility::TASK)
        .await
        .expect("the task runs");

    assert!(!s.reload(&s.early).await.conflict_eligibility);
    assert!(s.reload(&s.late).await.conflict_eligibility);
    assert!(
        s.reload(&s.beyond).await.conflict_eligibility,
        "day 44 entered the 30 day window on day 16"
    );
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 2);
}

/// Done when 3: `DocumentExpiring` with the booking of day 24 in `affected`
/// flags it once, and so does the real `wl-hr` re-emitting it as
/// `QualificationExpiring`; the next nightly run does not flag it again.
#[renox::test]
async fn an_expiring_document_flags_its_affected_booking_once() {
    let s = scene().await;
    s.licence_until(DOC, day(19));
    let event = DocumentExpiring {
        document_id: DocumentId(DOC),
        subject: s.subject.clone(),
        kind: LICENCE.to_owned(),
        expires_on: day(19),
        days_left: 19,
        offset: Some(30),
        affected: vec![AffectedGroup {
            guarded: "booking.book".to_owned(),
            records: vec![Scene::planned(&s.late)],
        }],
    };

    s.t.state().emit(event).await.unwrap();

    assert!(s.reload(&s.late).await.conflict_eligibility);
    assert!(!s.reload(&s.early).await.conflict_eligibility);
    assert_eq!(s.rows(&s.late).await.len(), 1);
    assert_eq!(s.t.events.conflicts.all().len(), 1);
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 1);

    let done = eligibility::look_ahead(s.t.state(), at(day(0), 2))
        .await
        .unwrap();
    assert_eq!(done.flagged, 0, "the night finds it flagged");
    assert_eq!(s.rows(&s.late).await.len(), 1);
    assert_eq!(s.t.events.conflicts.all().len(), 1);
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 1);
}

/// An event only names the bookings to look at: a booking in its list that
/// does pass at its start is not flagged.
#[renox::test]
async fn a_booking_that_passes_at_its_start_is_not_flagged_by_an_event() {
    let s = scene().await;
    // The licence lapses after the booking on day 24.
    s.licence_until(DOC, day(30));
    let event = DocumentExpiring {
        document_id: DocumentId(DOC),
        subject: s.subject.clone(),
        kind: LICENCE.to_owned(),
        expires_on: day(30),
        days_left: 30,
        offset: Some(30),
        affected: vec![AffectedGroup {
            guarded: "booking.book".to_owned(),
            records: vec![Scene::planned(&s.late)],
        }],
    };

    s.t.state().emit(event).await.unwrap();

    assert!(!s.reload(&s.late).await.conflict_eligibility);
    assert!(s.t.events.conflicts.all().is_empty());
}

/// `DocumentExpired` (and the `QualificationExpired` the real `wl-hr` makes
/// of it) flags the bookings planned after the expiry, in one
/// `BookingConflict`.
#[renox::test]
async fn an_expired_document_flags_the_bookings_planned_after_it() {
    let s = scene().await;
    s.licence_until(DOC, day(-1));
    let event = DocumentExpired {
        document: DocumentId(DOC),
        subject: s.subject.clone(),
        kind: LICENCE.to_owned(),
        expires_on: day(-1),
        blocks: Vec::new(),
        blocked: vec![BlockedGroup {
            guarded: "booking.book".to_owned(),
            mode: BlockMode::Block,
            open: Vec::new(),
            planned: vec![Scene::planned(&s.early), Scene::planned(&s.late)],
        }],
    };

    s.t.state().emit(event).await.unwrap();

    assert!(s.reload(&s.early).await.conflict_eligibility);
    assert!(s.reload(&s.late).await.conflict_eligibility);
    let events = s.t.events.conflicts.all();
    assert_eq!(events.len(), 1, "one event for the resource and the item");
    assert_eq!(
        events[0].bookings,
        vec![s.early.booking_id(), s.late.booking_id()]
    );
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 2);
}

/// Done when 3, the `wl-hr` side: `QualificationExpiring` flags the planned
/// booking it names (an entry already running is not touched), once; a second
/// delivery adds nothing.
#[renox::test]
async fn a_qualification_expiring_flags_the_planned_bookings_it_names() {
    let s = scene().await;
    s.licence_until(DOC, day(19));
    let employee = EmployeeId(s.doctor.employee_id.expect("an employee"));
    let event = QualificationExpiring {
        employee,
        qualification: None,
        kind: LICENCE.to_owned(),
        scope: Vec::new(),
        expires_on: day(19),
        days_left: 19,
        affected: vec![
            AffectedWork::of("booking.book", &Scene::planned(&s.late), false),
            AffectedWork::of("booking.book", &Scene::planned(&s.early), true),
            AffectedWork::of("invoice.issue", &Scene::planned(&s.beyond), false),
        ],
    };

    s.t.state().emit(event.clone()).await.unwrap();
    s.t.state().emit(event).await.unwrap();

    assert!(s.reload(&s.late).await.conflict_eligibility);
    assert!(!s.reload(&s.early).await.conflict_eligibility, "running");
    assert!(!s.reload(&s.beyond).await.conflict_eligibility, "not ours");
    assert_eq!(s.rows(&s.late).await.len(), 1);
    assert_eq!(s.t.events.conflicts.all().len(), 1);
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 1);
}

/// `QualificationExpired` flags the planned bookings it names within the
/// look-ahead; the booking it does not name stays, and one beyond the window
/// is left to the nightly run.
#[renox::test]
async fn a_qualification_expired_flags_the_planned_bookings_it_names() {
    let s = scene().await;
    s.licence_until(DOC, day(-1));
    let employee = EmployeeId(s.doctor.employee_id.expect("an employee"));

    s.t.state()
        .emit(QualificationExpired {
            employee,
            qualification: None,
            kind: LICENCE.to_owned(),
            scope: Vec::new(),
            roles: Vec::new(),
            expires_on: day(-1),
            affected: vec![
                AffectedWork::of("booking.book", &Scene::planned(&s.late), false),
                AffectedWork::of("booking.book", &Scene::planned(&s.beyond), false),
            ],
        })
        .await
        .unwrap();

    assert!(s.reload(&s.late).await.conflict_eligibility);
    assert!(
        !s.reload(&s.beyond).await.conflict_eligibility,
        "named, but beyond the look-ahead: the nightly run takes it later"
    );
    assert!(!s.reload(&s.early).await.conflict_eligibility, "not named");
    let events = s.t.events.conflicts.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cause, ConflictCause::Eligibility);
    assert_eq!(s.t.tasks.with_key("scheduling.eligibility:").len(), 1);
}

/// Done when 4: the renewed licence (valid to next year) is accepted; the
/// conflict clears, the task is resolved and the booking is unchanged.
#[renox::test]
async fn a_renewal_clears_the_conflict_and_leaves_the_booking() {
    let s = scene().await;
    s.licence_until(DOC, day(19));
    eligibility::look_ahead(s.t.state(), at(day(0), 2))
        .await
        .unwrap();
    assert!(s.reload(&s.late).await.conflict_eligibility);

    // A renewal that still ends before the booking clears nothing.
    s.licence_until(8, day(22));
    s.t.state()
        .emit(DocumentAccepted {
            document: DocumentId(8),
            subject: s.subject.clone(),
            kind: LICENCE.to_owned(),
            expires_on: Some(day(22)),
        })
        .await
        .unwrap();
    assert!(s.reload(&s.late).await.conflict_eligibility);
    assert!(!s.t.tasks.is_resolved(1));

    s.licence_until(9, day(400));
    s.t.state()
        .emit(DocumentAccepted {
            document: DocumentId(9),
            subject: s.subject.clone(),
            kind: LICENCE.to_owned(),
            expires_on: Some(day(400)),
        })
        .await
        .unwrap();

    let late = s.reload(&s.late).await;
    assert!(!late.conflict_eligibility);
    assert_eq!(late.status, s.late.status);
    assert_eq!(late.starts_at, s.late.starts_at);
    assert_eq!(
        late.resource_ids(s.t.db()).await.unwrap(),
        vec![s.doctor.id]
    );
    let rows = s.rows(&late).await;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_open());
    assert_eq!(rows[0].cleared_by.as_deref(), Some("renewed"));
    assert!(s.t.tasks.is_resolved(1));
}

/// The nightly run clears a conflict whose licence was renewed without an
/// event reaching the module.
#[renox::test]
async fn the_nightly_run_clears_what_was_renewed() {
    let s = scene().await;
    s.licence_until(DOC, day(19));
    eligibility::look_ahead(s.t.state(), at(day(0), 2))
        .await
        .unwrap();

    s.licence_until(8, day(400));
    let done = eligibility::look_ahead(s.t.state(), at(day(1), 2))
        .await
        .unwrap();

    assert_eq!(
        done,
        LookAhead {
            flagged: 0,
            cleared: 1
        }
    );
    assert!(!s.reload(&s.late).await.conflict_eligibility);
}

/// A booking with a `block` and an `eligibility` conflict keeps both causes;
/// the renewal clears only the second.
#[renox::test]
async fn clearing_the_eligibility_leaves_the_block() {
    let s = scene().await;
    s.licence_until(DOC, day(19));
    eligibility::look_ahead(s.t.state(), at(day(0), 2))
        .await
        .unwrap();
    let block = AvailabilityBlock::manual(
        s.doctor.id,
        s.late.starts_at - TimeDelta::hours(1),
        s.late.ends_at + TimeDelta::hours(1),
        "sick",
    );
    conflict::add_block(s.t.state(), block, renox::db::now())
        .await
        .expect("the block is added");
    let both = s.reload(&s.late).await;
    assert!(both.conflict_block && both.conflict_eligibility);

    s.licence_until(8, day(400));
    s.t.state()
        .emit(DocumentAccepted {
            document: DocumentId(8),
            subject: s.subject.clone(),
            kind: LICENCE.to_owned(),
            expires_on: Some(day(400)),
        })
        .await
        .unwrap();

    let late = s.reload(&s.late).await;
    assert!(late.conflict_block, "the block is still there");
    assert!(!late.conflict_eligibility);
    let open: Vec<String> = conflict::open_of(s.t.db(), late.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.cause)
        .collect();
    assert_eq!(open, vec!["block".to_owned()]);
}

/// The licence is role-bound: a resource that works in another role than the
/// licence covers fails at the booking's start, and so does a licence of
/// another branch.
#[renox::test]
async fn a_licence_of_another_role_or_branch_does_not_count() {
    let s = scene().await;
    let nurse = {
        let mut nurse = Resource::new("Nurse N", wl_scheduling::ResourceType::Person)
            .employee(s.doctor.employee_id.unwrap())
            .role("nurse")
            .at(&support::branch());
        nurse.save(s.t.db()).await.unwrap();
        s.t.open_all_week(&nurse).await;
        nurse
    };
    let elsewhere = {
        let mut other = Resource::new("Dr A, branch 3", wl_scheduling::ResourceType::Person)
            .employee(s.doctor.employee_id.unwrap())
            .role("practitioner")
            .at(&Subject::new("test.branch", 3));
        other.save(s.t.db()).await.unwrap();
        other
    };

    for resource in [&nurse, &elsewhere] {
        let failures = eligibility::failures(s.t.state(), &s.service, resource, s.late.starts_at)
            .await
            .unwrap();
        assert_eq!(failures.len(), 1, "{}", resource.name);
        assert_eq!(failures[0].kind, LICENCE);
    }
    let fine = eligibility::failures(s.t.state(), &s.service, &s.doctor, s.late.starts_at)
        .await
        .unwrap();
    assert!(fine.is_empty());
}

/// The providers wl-documents asks: the bookings of a resource dated from a
/// day (`FutureWork`), and a booking's resources' employees (`BlockSubjects`)
/// for `booking.book` and `booking.check_in`.
#[renox::test]
async fn the_module_registers_its_future_work_and_block_subjects() {
    let s = scene().await;
    let entry = |guarded: &str, kind: &str| BlockEntry {
        kind: kind.to_owned(),
        guarded: guarded.to_owned(),
        mode: BlockMode::Block,
        override_mode: OverrideMode::Never,
        condition: None,
        locked: true,
    };

    for guarded in ["booking.book", "booking.check_in"] {
        let work = future_work(guarded).expect("a FutureWork provider");
        let from_18 = work
            .planned_from(s.t.state(), &entry(guarded, LICENCE), &s.subject, day(18))
            .await
            .unwrap();
        let records: Vec<Subject> = from_18.iter().map(|r| r.record.clone()).collect();
        assert_eq!(records, vec![s.late.subject(), s.beyond.subject()]);
        assert_eq!(from_18[0].date, day(24));
        let from_25 = work
            .planned_from(s.t.state(), &entry(guarded, LICENCE), &s.subject, day(25))
            .await
            .unwrap();
        assert_eq!(from_25.len(), 1);
        // A kind the service does not need lists nothing.
        let other = work
            .planned_from(
                s.t.state(),
                &entry(guarded, "fire_card"),
                &s.subject,
                day(0),
            )
            .await
            .unwrap();
        assert!(other.is_empty());

        let subjects = block_subjects(guarded).expect("a BlockSubjects provider");
        let found = subjects
            .subjects(s.t.state(), &entry(guarded, LICENCE), s.late.id)
            .await
            .unwrap();
        assert_eq!(found, vec![s.subject.clone()]);
        assert!(
            subjects
                .subjects(s.t.state(), &entry(guarded, "fire_card"), s.late.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert!(block_subjects("booking.book").unwrap().bookable());
    assert!(!block_subjects("booking.check_in").unwrap().bookable());
}

/// The nightly run is a kernel rule run of `SCH.RUL.05` under the schedule
/// subject, so that a vertical that marks the rule compliance-critical (05)
/// is alerted when it fails and the kernel finds that the night ran.
#[renox::test]
async fn the_nightly_run_is_recorded_as_a_rule_run_of_sch_rul_05() {
    use wl_kernel::automation::{RuleRun, RuleRunStatus};

    let s = scene().await;
    s.t.app
        .kernel()
        .call(eligibility::TASK, Vec::<String>::new())
        .await
        .expect("the command runs");
    let runs = RuleRun::query()
        .where_(RuleRun::RULE.eq(eligibility::RULE))
        .get(s.t.db())
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RuleRunStatus::Succeeded);
    assert_eq!(eligibility::RULE, "SCH.RUL.05");
}
