//! BL-CLN-12 (#360): `CLN.RUL.01`, a practitioner's licence or branch
//! privileges expire. The block is wl-hr's and wl-scheduling's (real modules
//! here, with the document desk of wl-scheduling's support); the app tells
//! the clinic manager of the branch and the owner, once each.
//!
//! The days are relative to today (the services take the real clock through
//! the routes): the licence of Dr K is valid through day 10, as through 30
//! November in the spec scenario, with four appointments from day 12 on.

mod support;

use std::time::Duration;

use renox::auth::permissions::{self, Scope};
use renox::prelude::*;
use wl_kernel::automation::RunOutcome;
use wl_kernel::compliance::{ComplianceAlert, is_critical};
use wl_kernel::documents::{
    BlockMode, BlockedGroup, DocumentExpired, PlannedRecord, dubai_date,
};
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_kernel::workforce::{AffectedWork, QualificationExpired};
use wl_kernel::{DocumentId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{Booking, Refusal, Resource};

use clinics::licences::{self, LICENCE_EXPIRED, LicenceAlert, RULE};
use clinics::permissions::{CLINIC_MANAGER_ROLE, OWNER_ROLE};
use support::{
    Clinic, LICENCE, Messages, PATIENT, PRIVILEGES, at, boot, branch, day, other_branch,
};

/// The licence document of Dr K.
const DOCUMENT: i64 = 61;

fn scope_of(subject: &Subject) -> Scope {
    Scope::new(&subject.subject_type, subject.subject_id)
}

/// The clinic manager of each branch and the owner.
struct Staff {
    manager: User,
    other_manager: User,
    owner: User,
}

async fn staff(t: &Clinic) -> Staff {
    permissions::define_role(t.db(), CLINIC_MANAGER_ROLE, &[])
        .await
        .unwrap();
    permissions::define_role(t.db(), OWNER_ROLE, &[])
        .await
        .unwrap();
    let manager = t.user("Mona", &[]).await;
    manager
        .assign_role_in(t.db(), CLINIC_MANAGER_ROLE, &scope_of(&branch()))
        .await
        .unwrap();
    let other_manager = t.user("Omar", &[]).await;
    other_manager
        .assign_role_in(t.db(), CLINIC_MANAGER_ROLE, &scope_of(&other_branch()))
        .await
        .unwrap();
    let owner = t.user("Olga", &[]).await;
    owner.assign_role(t.db(), OWNER_ROLE).await.unwrap();
    Staff {
        manager,
        other_manager,
        owner,
    }
}

fn planned(booking: &Booking) -> PlannedRecord {
    PlannedRecord {
        record: booking.subject(),
        date: dubai_date(booking.starts_at),
        label: booking.number.clone(),
        scope: None,
    }
}

/// `DocumentExpired` for `kind` of the practitioner, listing `bookings` as
/// planned after the expiry.
fn expired(t: &Clinic, doctor: &Resource, kind: &str, bookings: &[&Booking]) -> DocumentExpired {
    DocumentExpired {
        document: DocumentId(DOCUMENT),
        subject: t.employee_id(doctor).subject(),
        kind: kind.to_owned(),
        expires_on: day(10),
        blocks: Vec::new(),
        blocked: vec![BlockedGroup {
            guarded: "booking.book".to_owned(),
            mode: BlockMode::Block,
            open: Vec::new(),
            planned: bookings.iter().map(|b| planned(b)).collect(),
        }],
    }
}

/// What the clinics app told, by type.
fn told(t: &Clinic) -> Vec<NotifyRequest> {
    t.messages
        .sent()
        .into_iter()
        .filter(|request| request.type_key == LICENCE_EXPIRED)
        .collect()
}

fn users_of(requests: &[NotifyRequest]) -> Vec<i64> {
    let mut ids: Vec<i64> = requests
        .iter()
        .flat_map(|r| r.recipients.iter())
        .filter_map(|recipient| match recipient {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// Done when 1 and 2: Dr K's licence is valid through day 10 and four
/// appointments follow it. After the expiry a new booking from day 11 is
/// refused (no override), the manager of the branch and the owner each get
/// one message naming four appointments (the manager of the other branch
/// none), the four are in conflict, and on day 10 itself Dr K can still be
/// checked in.
#[renox::test]
async fn the_manager_and_the_owner_are_told_once_and_the_bookings_are_in_conflict() {
    let t = boot().await;
    let people = staff(&t).await;
    let doctor = t.practitioner("Dr K", 61, None).await;
    let service_type = t.consultation(&[&doctor]).await;
    let last_day = t.book(&service_type, &doctor, at(day(10), 9)).await;
    let mut december = Vec::new();
    for days in [12, 13, 14, 15] {
        december.push(t.book(&service_type, &doctor, at(day(days), 10)).await);
    }
    // The licence runs out after day 10.
    t.docs
        .accepted_until(&t.employee_id(&doctor).subject(), LICENCE, DOCUMENT, Some(day(10)));
    let refs: Vec<&Booking> = december.iter().collect();

    t.state()
        .emit(expired(&t, &doctor, LICENCE, &refs))
        .await
        .unwrap();

    // One message each to the manager of Branch 1 and the owner.
    let messages = told(&t);
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_eq!(
        users_of(&messages),
        {
            let mut both = vec![people.manager.id, people.owner.id];
            both.sort_unstable();
            both
        },
        "the manager of the other branch is not told"
    );
    assert!(!users_of(&messages).contains(&people.other_manager.id));
    for request in &messages {
        assert_eq!(request.data["count"].as_i64(), Some(4));
        let body = Messages::body(request);
        assert!(body.contains("practitioner licence"), "{body}");
        assert!(body.contains("Appointments affected: 4"), "{body}");
        assert!(body.contains("test.branch 2"), "{body}");
        assert!(body.contains("employee"), "{body}");
    }
    let employee = t.employee_id(&doctor).0;
    let rows = LicenceAlert::of_expiry(t.db(), employee, LICENCE, day(10))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.appointments == 4));

    // The four appointments are in conflict (`SCH.RUL.05`); the one on the
    // last valid day is not, and nothing was cancelled.
    for booking in &december {
        let again = t.booking(booking.id).await;
        assert!(again.conflict_eligibility, "{} is in conflict", again.number);
        assert_eq!(again.status, booking.status);
    }
    assert!(!t.booking(last_day.id).await.conflict_eligibility);
    let flagged: usize = t.events.conflicts.all().iter().map(|e| e.bookings.len()).sum();
    assert_eq!(flagged, 4);

    // A new booking after the expiry is refused, whoever asks.
    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, service_type.id, &[doctor.id], at(day(11), 9))
            .for_subject(PATIENT.subject()),
        Actor::staff(1),
        support::scheduling::now(),
    )
    .await;
    let Err(Refusal::Ineligible(text)) = refused else {
        panic!("the booking is refused: {refused:?}");
    };
    assert!(text.contains("practitioner licence expired on"), "{text}");

    // On the last valid day the patient can still be checked in; the day
    // after, a check-in is refused.
    service::check_in(t.state(), last_day.id, Actor::staff(1), at(day(10), 9))
        .await
        .expect("valid through the expiry day");
    let late = service::check_in(t.state(), december[0].id, Actor::staff(1), at(day(11), 9)).await;
    assert!(matches!(late, Err(Refusal::Ineligible(_))), "{late:?}");

    // The same expiry again tells nobody a second time.
    t.state()
        .emit(expired(&t, &doctor, LICENCE, &refs))
        .await
        .unwrap();
    assert_eq!(told(&t).len(), 2);
    let event = t.expired.all()[0].clone();
    let (outcome, again) = licences::on_expired(t.state(), &event)
        .await
        .unwrap()
        .expect("the licence is a kind of the rule");
    assert_eq!(outcome, RunOutcome::Skipped);
    assert!(again.users.is_empty());
    assert_eq!(told(&t).len(), 2);
}

/// Done when 3: Dr M works at Branch 2, where her privileges expire; Branch 1
/// is not told and Branch 1's bookings (Dr K's) still work. The owner is told,
/// the message names Branch 2 and its manager only.
#[renox::test]
async fn privileges_expired_at_one_branch_leave_the_other_alone() {
    let t = boot().await;
    let people = staff(&t).await;
    let at_one = t.practitioner("Dr K", 61, None).await;
    let at_two = t
        .practitioner_at("Dr M", 62, &other_branch(), None)
        .await;
    let service_type = t.consultation(&[&at_one, &at_two]).await;
    let kept = t.book(&service_type, &at_one, at(day(12), 10)).await;
    let hers = t.book(&service_type, &at_two, at(day(12), 10)).await;
    t.docs.accepted_until(
        &t.employee_id(&at_two).subject(),
        PRIVILEGES,
        62,
        Some(day(10)),
    );

    t.state()
        .emit(expired(&t, &at_two, PRIVILEGES, &[&hers]))
        .await
        .unwrap();

    let messages = told(&t);
    let mut expected = vec![people.other_manager.id, people.owner.id];
    expected.sort_unstable();
    assert_eq!(users_of(&messages), expected);
    assert!(!users_of(&messages).contains(&people.manager.id));
    for request in &messages {
        assert_eq!(request.data["count"].as_i64(), Some(1));
        assert!(Messages::body(request).contains("test.branch 3"));
    }
    assert!(t.booking(hers.id).await.conflict_eligibility);
    assert!(!t.booking(kept.id).await.conflict_eligibility);
    // Branch 1 stays bookable; Dr M no longer is, at Branch 2.
    let ok = service::book(
        t.state(),
        NewBooking::new(PATIENT, service_type.id, &[at_one.id], at(day(13), 9))
            .for_subject(PATIENT.subject()),
        Actor::staff(1),
        support::scheduling::now(),
    )
    .await;
    assert!(ok.is_ok(), "{ok:?}");
    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, service_type.id, &[at_two.id], at(day(13), 9))
            .for_subject(PATIENT.subject()),
        Actor::staff(1),
        support::scheduling::now(),
    )
    .await;
    assert!(matches!(refused, Err(Refusal::Ineligible(_))), "{refused:?}");
}

/// A kind the rule is not about (a driving licence) tells nobody.
#[renox::test]
async fn another_kind_is_not_the_rules_business() {
    let t = boot().await;
    staff(&t).await;
    let doctor = t.practitioner("Dr K", 61, None).await;
    let event = QualificationExpired {
        employee: t.employee_id(&doctor),
        qualification: None,
        kind: "driving_licence".to_owned(),
        scope: vec![branch()],
        roles: Vec::new(),
        expires_on: day(10),
        affected: Vec::new(),
    };

    let done = licences::on_expired(t.state(), &event).await.unwrap();

    assert!(done.is_none());
    assert!(told(&t).is_empty());
}

/// Done when 4: a run that fails is tried again (1, 5, 15 minutes and an
/// hour later) and, at the fifth failure, alerts the owner role and the
/// company admin (`PLT.RUL.04`); the rule is compliance-critical.
#[renox::test]
async fn a_failing_run_alerts_the_owner() {
    let t = boot().await;
    let people = staff(&t).await;
    let doctor = t.practitioner("Dr K", 61, None).await;
    assert!(is_critical(RULE));
    assert!(is_critical("DOC.RUL.03"));
    renox::db::sql("ALTER TABLE \"clinics_licence_alerts\" RENAME TO \"clinics_licence_alerts_off\"")
        .execute(t.db())
        .await
        .unwrap();
    let event = QualificationExpired {
        employee: t.employee_id(&doctor),
        qualification: None,
        kind: LICENCE.to_owned(),
        scope: vec![branch()],
        roles: vec!["practitioner".to_owned()],
        expires_on: day(10),
        affected: vec![AffectedWork {
            guarded: "booking.book".to_owned(),
            record: Subject::new("scheduling.booking", 1),
            date: day(12),
            label: "BKG-1".to_owned(),
            open: false,
        }],
    };
    let alerted = |user: i64| {
        t.messages.sent().iter().any(|r| {
            r.type_key == "kernel.compliance_alert"
                && r.recipients.contains(&NotifyRecipient::User { id: user })
        })
    };

    for wait in [60_u64, 300, 900, 3600] {
        let outcome = t
            .app
            .at_travelled_time(licences::on_expired(t.state(), &event))
            .await;
        let (outcome, _) = outcome.unwrap().unwrap();
        assert_eq!(outcome, RunOutcome::Retrying);
        assert!(!alerted(people.owner.id), "not before the last attempt");
        t.app.travel(Duration::from_secs(wait));
    }
    let last = t
        .app
        .at_travelled_time(licences::on_expired(t.state(), &event))
        .await;
    let (outcome, _) = last.unwrap().unwrap();
    assert_eq!(outcome, RunOutcome::Failed);

    let alerts: Vec<ComplianceAlert> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].owner_role, OWNER_ROLE);
    assert!(alerted(people.owner.id), "the owner is told");
    assert!(told(&t).is_empty(), "nobody was told the licence expired");
}
