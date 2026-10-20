//! BL-HRM-01 (#191): `qualification_valid` (X-13) answers through the
//! wl-documents guard plus role and scope, and `HRM.RUL.10` emits
//! `QualificationExpired` with the blocked work when a blocking kind expires.

mod support;

use wl_hr::qualification::{self, NewQualification, qualification_valid, qualification_valid_for};
use wl_hr::{Employee, EmployeeStatus};
use wl_kernel::documents::{AffectedGroup, PlannedRecord};
use wl_kernel::documents::{BlockMode, BlockedGroup, DocumentExpired, DocumentExpiring};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::qualifications::{Qualifications, qualifications};
use wl_kernel::{DocumentId, Subject};

use support::{Booted, branch, nov, other_branch};

const LICENCE: &str = "practitioner_licence";

/// Dr A, a practitioner at Branch 1 (the tests' branch B) whose licence, the
/// document `7`, is valid through 2026-11-20.
async fn dr_a(t: &Booted) -> Employee {
    let a = t.employee_in(61, EmployeeStatus::Active).await;
    qualification::record(
        t.db(),
        NewQualification::new(a.employee_id(), LICENCE)
            .role("practitioner")
            .scope(branch())
            .number("DHA-1")
            .valid_to(nov(20)),
    )
    .await
    .unwrap();
    t.docs
        .accepted_until(&a.subject(), LICENCE, 7, Some(nov(20)));
    a
}

/// A booking planned for `day`.
fn booking(id: i64, day: u32) -> PlannedRecord {
    PlannedRecord {
        record: Subject::new("scheduling.booking", id),
        date: nov(day),
        label: format!("Booking {id}"),
        scope: None,
    }
}

/// The `DocumentExpired` of Dr A's licence with two bookings on 2026-11-24.
fn expired(a: &Employee) -> DocumentExpired {
    DocumentExpired {
        document: DocumentId(7),
        subject: a.subject(),
        kind: LICENCE.into(),
        expires_on: nov(20),
        blocks: Vec::new(),
        blocked: vec![BlockedGroup {
            guarded: "scheduling.booking.confirm".into(),
            mode: BlockMode::Block,
            open: Vec::new(),
            planned: vec![booking(31, 24), booking(32, 24)],
        }],
    }
}

/// Valid on the expiry date, not the day after; the role and scope must be
/// the licence's.
#[renox::test]
async fn hrm01_the_licence_is_valid_through_its_expiry_date_for_its_role_and_scope() {
    let t = support::boot().await;
    let a = dr_a(&t).await;
    let id = a.employee_id();
    let asked = |day: u32, role: &'static str, scope: Subject| {
        let state = t.state().clone();
        async move {
            qualification_valid_for(&state, id, LICENCE, nov(day), role, &scope)
                .await
                .unwrap()
        }
    };
    assert!(asked(20, "practitioner", branch()).await);
    assert!(!asked(21, "practitioner", branch()).await);
    // The past counts too.
    assert!(asked(1, "practitioner", branch()).await);
    // Another role does not.
    assert!(!asked(20, "nurse", branch()).await);
}

/// A scope the licence does not list answers *no* on any date, and so does a
/// role-bound kind asked without role and scope.
#[renox::test]
async fn hrm01_another_scope_or_no_role_and_scope_answers_no() {
    let t = support::boot().await;
    let a = dr_a(&t).await;
    let id = a.employee_id();
    for day in [1, 20, 21, 30] {
        assert!(
            !qualification_valid_for(
                t.state(),
                id,
                LICENCE,
                nov(day),
                "practitioner",
                &other_branch()
            )
            .await
            .unwrap(),
            "Branch 2 on {day}"
        );
    }
    assert!(
        !qualification_valid(t.state(), id, LICENCE, nov(20))
            .await
            .unwrap()
    );
    // The same through the kernel service other modules call.
    let service = qualifications(t.state());
    assert!(
        !service
            .qualification_valid(t.state(), id, LICENCE, nov(20), None, None)
            .await
            .unwrap()
            .valid
    );
    let found = service
        .qualification_valid(
            t.state(),
            id,
            LICENCE,
            nov(20),
            Some("practitioner"),
            Some(&branch()),
        )
        .await
        .unwrap();
    assert!(found.valid);
    assert!(found.number.is_some());
    assert!(
        !service
            .qualification_valid(
                t.state(),
                id,
                LICENCE,
                nov(20),
                Some("practitioner"),
                Some(&other_branch())
            )
            .await
            .unwrap()
            .valid
    );
}

/// A kind with no role or scope on the qualification (or an employee
/// document with no qualification record, a freelancer's permit) is answered
/// by the document alone.
#[renox::test]
async fn hrm01_a_kind_that_is_not_role_bound_follows_the_document() {
    let t = support::boot().await;
    let f = t.employee_in(62, EmployeeStatus::Active).await;
    let id = f.employee_id();
    assert!(
        !qualification_valid(t.state(), id, "freelance_permit", nov(10))
            .await
            .unwrap(),
        "missing"
    );
    t.docs
        .accepted_until(&f.subject(), "freelance_permit", 3, Some(nov(15)));
    assert!(
        qualification_valid(t.state(), id, "freelance_permit", nov(15))
            .await
            .unwrap()
    );
    assert!(
        !qualification_valid(t.state(), id, "freelance_permit", nov(16))
            .await
            .unwrap()
    );
    // Without the hr module's service the answer is no, whatever the date.
    let nobody = wl_kernel::NoQualifications;
    assert!(
        !nobody
            .qualification_valid(t.state(), id, "freelance_permit", nov(15), None, None)
            .await
            .unwrap()
            .valid
    );
}

/// `DocumentExpired` with two bookings on 2026-11-24 emits
/// `QualificationExpired` once with those two, and tells the manager of
/// Branch 1 and HR.
#[renox::test]
async fn hrm01_an_expired_licence_emits_one_event_and_tells_the_manager_and_hr() {
    let t = support::boot().await;
    let a = dr_a(&t).await;
    let manager = t.user_in_branch("Mia", "manager").await;
    let hana = t.user_with("Hana", "hr").await;
    let elsewhere = t.user("Eve").await;
    elsewhere
        .assign_role_in(
            t.db(),
            "manager",
            &support::branch_scope_of(&other_branch()),
        )
        .await
        .unwrap();

    t.state().emit(expired(&a)).await.unwrap();

    let events = t.qualification_expired.all();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.employee, a.employee_id());
    assert_eq!(event.kind, LICENCE);
    assert_eq!(event.expires_on, nov(20));
    assert_eq!(event.scope, vec![branch()]);
    assert_eq!(event.roles, vec!["practitioner".to_owned()]);
    assert!(event.qualification.is_some());
    let records: Vec<&Subject> = event.affected.iter().map(|w| &w.record).collect();
    assert_eq!(
        records,
        [
            &Subject::new("scheduling.booking", 31),
            &Subject::new("scheduling.booking", 32)
        ]
    );
    assert!(event.affected.iter().all(|w| w.date == nov(24) && !w.open));

    let told = t.notices.of_type("hr.qualification_expired");
    assert_eq!(told.len(), 1);
    let to: Vec<i64> = told[0]
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(to.contains(&manager.id), "{to:?}");
    assert!(to.contains(&hana.id), "{to:?}");
    assert!(!to.contains(&elsewhere.id), "{to:?}");
    assert_eq!(told[0].data["count"], 2);

    // Once per qualification and expiry date: the same event again emits
    // nothing.
    t.state().emit(expired(&a)).await.unwrap();
    assert_eq!(t.qualification_expired.all().len(), 1);
    assert_eq!(t.notices.of_type("hr.qualification_expired").len(), 1);
    // The guard says no, from the document.
    assert!(
        !qualification_valid_for(
            t.state(),
            a.employee_id(),
            LICENCE,
            nov(21),
            "practitioner",
            &branch()
        )
        .await
        .unwrap()
    );
}

/// With no manager in Branch 1 only HR is told.
#[renox::test]
async fn hrm01_with_no_manager_only_hr_is_told() {
    let t = support::boot().await;
    let a = dr_a(&t).await;
    let hana = t.user_with("Hana", "hr").await;
    t.state().emit(expired(&a)).await.unwrap();
    let told = t.notices.of_type("hr.qualification_expired");
    assert_eq!(told.len(), 1);
    let to: Vec<&NotifyRecipient> = told[0].recipients.iter().collect();
    assert_eq!(to, [&NotifyRecipient::User { id: hana.id }]);
}

/// An employee document of a blocking kind with no qualification record
/// still gives one event.
#[renox::test]
async fn hrm01_an_employee_document_without_a_record_gives_one_event() {
    let t = support::boot().await;
    let f = t.employee_in(62, EmployeeStatus::Active).await;
    t.state()
        .emit(DocumentExpired {
            document: DocumentId(3),
            subject: f.subject(),
            kind: "freelance_permit".into(),
            expires_on: nov(15),
            blocks: Vec::new(),
            blocked: Vec::new(),
        })
        .await
        .unwrap();
    let events = t.qualification_expired.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].qualification, None);
    assert!(events[0].affected.is_empty());
    // A document of another subject type is not ours.
    t.state()
        .emit(DocumentExpired {
            document: DocumentId(4),
            subject: Subject::new("property.lease", 1),
            kind: "freelance_permit".into(),
            expires_on: nov(15),
            blocks: Vec::new(),
            blocked: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(t.qualification_expired.all().len(), 1);
}

/// A renewal that is still `pending_review` leaves the guard at *no*; once
/// it is accepted the guard answers *yes* for 2026-11-25.
#[renox::test]
async fn hrm01_a_pending_renewal_answers_no_until_it_is_accepted() {
    let t = support::boot().await;
    let a = dr_a(&t).await;
    let id = a.employee_id();
    let ask = |day: u32| {
        let state = t.state().clone();
        async move {
            qualification_valid_for(&state, id, LICENCE, nov(day), "practitioner", &branch())
                .await
                .unwrap()
        }
    };
    t.docs.pending_review(&a.subject(), LICENCE, 8);
    assert!(!ask(25).await, "pending review");

    t.docs.accepted_until(
        &a.subject(),
        LICENCE,
        8,
        Some(nov(30) + renox::chrono::Days::new(365)),
    );
    assert!(ask(25).await, "accepted");
    assert!(ask(21).await);
}

/// `DocumentExpiring` gives `QualificationExpiring` with its `affected` list
/// unchanged.
#[renox::test]
async fn hrm01_expiring_carries_the_affected_list_unchanged() {
    let t = support::boot().await;
    let a = dr_a(&t).await;
    t.state()
        .emit(DocumentExpiring {
            document_id: DocumentId(7),
            subject: a.subject(),
            kind: LICENCE.into(),
            expires_on: nov(20),
            days_left: 14,
            offset: Some(14),
            affected: vec![AffectedGroup {
                guarded: "scheduling.booking.confirm".into(),
                records: vec![booking(41, 25), booking(42, 26)],
            }],
        })
        .await
        .unwrap();
    let events = t.qualification_expiring.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].days_left, 14);
    assert_eq!(events[0].scope, vec![branch()]);
    let labels: Vec<&str> = events[0]
        .affected
        .iter()
        .map(|w| w.label.as_str())
        .collect();
    assert_eq!(labels, ["Booking 41", "Booking 42"]);
    assert!(t.notices.of_type("hr.qualification_expired").is_empty());
}
