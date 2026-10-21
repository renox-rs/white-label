//! Escalating a conflict that is still open, and keeping reminders and
//! no-shows away from a conflicted booking (#173, BL-SCH-06, `SCH.RUL.06`,
//! `SCH.RUL.01`, `SCH.RUL.02`).

mod support;

use renox::auth::permissions::{self, Scope};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_kernel::workforce::{EmployeeId, LeaveApproved, LeaveCancelled};
use wl_scheduling::permissions as perm;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityBlock, Booking, BookingStatus, Refusal, Resource, ServiceType, WlScheduling,
    conflict, rules,
};

use support::{Booked, Messages, OTHER_PATIENT, PATIENT, boot, boot_with, dubai, now};

const DAY: &str = "2030-03-12";
const PAST: &str = "2026-01-12";
const EMPLOYEE: i64 = 7;
const BRANCH: i64 = 5;
const OTHER_BRANCH: i64 = 6;

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

fn sick(request: i64) -> LeaveApproved {
    LeaveApproved {
        employee: EmployeeId(EMPLOYEE),
        request,
        leave_type: "sick".to_owned(),
        from: date(DAY),
        to: date(DAY),
    }
}

fn branch(id: i64) -> Subject {
    Subject::new("scheduling.branch", id)
}

/// A branch is a scope of its own.
fn register_branches() {
    wl_kernel::scopes::register("scheduling.branch", |_db, id| {
        Box::pin(async move { Ok(Scope::new("test_branch", id)) })
    });
}

fn scope(id: i64) -> Scope {
    Scope::new("test_branch", id)
}

/// A user holding `role` (with `grants`) in `scope` only.
async fn holder(t: &Booked, name: &str, role: &str, grants: &[&str], scope: &Scope) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.db(), name, &email, "password123")
        .await
        .unwrap();
    permissions::define_role(t.db(), role, grants).await.unwrap();
    user.assign_role_in(t.db(), role, scope).await.unwrap();
    user
}

/// Dr A (the employee behind her) and Dr B, both in branch 5 and delivering a
/// 30-minute service with a deposit of AED 200.
struct Scene {
    a: Resource,
    b: Resource,
    service: ServiceType,
}

async fn scene(t: &Booked) -> Scene {
    register_branches();
    let mut a = t.doctor("Dr A").await;
    a.employee_id = Some(EMPLOYEE);
    a.location_type = Some("scheduling.branch".to_owned());
    a.location_id = Some(BRANCH);
    a.save(t.db()).await.unwrap();
    let mut b = t.doctor("Dr B").await;
    b.location_type = Some("scheduling.branch".to_owned());
    b.location_id = Some(BRANCH);
    b.save(t.db()).await.unwrap();
    let service = t
        .service_with("consultation", &[&a, &b], |service| {
            service.deposit_amount = 20_000;
        })
        .await;
    Scene { a, b, service }
}

/// A booking in branch 5.
async fn book_at(
    t: &Booked,
    s: &Scene,
    resource: &Resource,
    day: &str,
    hour: u32,
    minute: u32,
    made_at: renox::db::DateTime,
) -> Booking {
    service::book(
        t.state(),
        NewBooking::new(PATIENT, s.service.id, &[resource.id], dubai(day, hour, minute))
            .at(branch(BRANCH)),
        Actor::staff(1),
        made_at,
    )
    .await
    .expect("the booking is made")
}

fn of_type(t: &Booked, key: &str) -> Vec<NotifyRequest> {
    t.messages
        .sent()
        .into_iter()
        .filter(|request| request.type_key == key)
        .collect()
}

/// The conflicted booking at 15:00 and an ordinary one with Dr B at 15:00.
/// At 13:00 the branch manager is told, the party gets the change message
/// with the three links and no reason, the 2 h reminder of the conflicted
/// booking is not sent (nor the 24 h one a run before), and the ordinary
/// booking is reminded each time. A run again sends nothing more.
#[renox::test]
async fn an_open_conflict_is_escalated_two_hours_before_and_its_reminder_is_suppressed() {
    let t = boot_with(WlScheduling::new().manager_role("branch_manager")).await;
    let s = scene(&t).await;
    let manager = holder(&t, "Mona", "branch_manager", &[perm::VIEW], &scope(BRANCH)).await;
    holder(
        &t,
        "Desk",
        "scoped_desk",
        &[perm::VIEW, perm::MANAGE],
        &scope(BRANCH),
    )
    .await;
    let booking = book_at(&t, &s, &s.a, DAY, 15, 0, now()).await;
    let control = book_at(&t, &s, &s.b, DAY, 15, 0, now()).await;
    t.state().emit(sick(601)).await.unwrap();
    assert!(t.reload(booking.id).await.conflict_block);

    // A minute before the window only the 24 h reminders are due.
    let early = rules::run(t.state(), dubai(DAY, 12, 59)).await.unwrap();
    assert_eq!(early.escalations, 0, "not yet two hours before");
    assert!(of_type(&t, "scheduling.conflict_escalated").is_empty());
    assert!(of_type(&t, "scheduling.bookings_changed").is_empty());

    let sent = rules::run(t.state(), dubai(DAY, 13, 0)).await.unwrap();

    assert_eq!(sent.escalations, 1);
    assert_eq!(sent.suppressed, 1, "the conflicted booking's reminder");
    assert_eq!(sent.reminders, 1, "the ordinary booking's reminder");
    let told = of_type(&t, "scheduling.conflict_escalated");
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        [NotifyRecipient::User { id: manager.id }],
        "the manager role, not everyone who may manage bookings"
    );
    assert!(Messages::body(&told[0]).contains(&booking.number));
    let changes = of_type(&t, "scheduling.bookings_changed");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].recipients, [NotifyRecipient::Party { id: PATIENT }]);
    let body = Messages::body(&changes[0]);
    assert!(body.contains(&format!("/scheduling/links/{}/accept", booking.id)));
    assert!(body.contains(&format!("/scheduling/links/{}/reschedule", booking.id)));
    assert!(body.contains(&format!(
        "/scheduling/links/{}/cancel/resource_unavailable",
        booking.id
    )));
    assert!(!body.to_lowercase().contains("sick"), "{body}");
    let reminders = of_type(&t, "scheduling.booking_reminder");
    assert_eq!(reminders.len(), 2, "the ordinary booking, 24 h and 2 h");
    assert!(reminders.iter().all(|r| r.subject == control.subject()));
    assert!(reminders.iter().all(|r| r.subject != booking.subject()));

    let again = rules::run(t.state(), dubai(DAY, 13, 5)).await.unwrap();
    assert_eq!(again.escalations, 0, "one escalation per conflict");
    assert_eq!(again.reminders + again.suppressed, 0);
    assert_eq!(of_type(&t, "scheduling.conflict_escalated").len(), 1);
    assert_eq!(of_type(&t, "scheduling.bookings_changed").len(), 1);
    assert_eq!(of_type(&t, "scheduling.booking_reminder").len(), 2);
}

/// A conflict raised at 14:10 for 15:00 is already inside the two hours: it is
/// escalated at 14:10, and not again.
#[renox::test]
async fn a_conflict_raised_inside_the_window_is_escalated_at_once() {
    let t = boot().await;
    let s = scene(&t).await;
    let booking = book_at(&t, &s, &s.a, DAY, 15, 0, now()).await;

    let flagged = conflict::leave_approved(t.state(), &sick(602), dubai(DAY, 14, 10))
        .await
        .unwrap();

    assert_eq!(flagged.len(), 1);
    assert_eq!(of_type(&t, "scheduling.bookings_changed").len(), 1);
    assert_eq!(
        of_type(&t, "scheduling.bookings_changed")[0].subject,
        booking.subject()
    );
    assert_eq!(of_type(&t, "scheduling.conflict_escalated").len(), 0);
    // Nobody holds a manager role or manages bookings in the branch, so the
    // administrators would be told; there are none here.
    let later = rules::run(t.state(), dubai(DAY, 14, 15)).await.unwrap();
    assert_eq!(later.escalations, 0);
    assert_eq!(of_type(&t, "scheduling.bookings_changed").len(), 1);
}

/// With nobody in the manager role in the branch, the users with
/// `bookings.manage` in that branch are told, and those of another branch are
/// not.
#[renox::test]
async fn without_a_manager_the_users_who_manage_bookings_there_are_told() {
    let t = boot_with(WlScheduling::new().manager_role("branch_manager")).await;
    let s = scene(&t).await;
    holder(
        &t,
        "Elsewhere",
        "branch_manager",
        &[perm::VIEW],
        &scope(OTHER_BRANCH),
    )
    .await;
    let desk = holder(
        &t,
        "Desk",
        "scoped_desk",
        &[perm::VIEW, perm::MANAGE],
        &scope(BRANCH),
    )
    .await;
    holder(
        &t,
        "Otherdesk",
        "other_scoped_desk",
        &[perm::VIEW, perm::MANAGE],
        &scope(OTHER_BRANCH),
    )
    .await;
    book_at(&t, &s, &s.a, DAY, 15, 0, now()).await;
    t.state().emit(sick(603)).await.unwrap();

    let sent = rules::run(t.state(), dubai(DAY, 13, 0)).await.unwrap();

    assert_eq!(sent.escalations, 1);
    let told = of_type(&t, "scheduling.conflict_escalated");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, [NotifyRecipient::User { id: desk.id }]);
}

/// At 15:15 the no-show sweep leaves a conflicted booking alone, and a staff
/// `mark_no_show` is refused by the machine.
#[renox::test]
async fn a_conflicted_booking_is_not_marked_no_show_by_the_sweep_or_by_staff() {
    let t = boot().await;
    let s = scene(&t).await;
    let booking = book_at(&t, &s, &s.a, DAY, 15, 0, now()).await;
    t.state().emit(sick(604)).await.unwrap();

    let swept = rules::run(t.state(), dubai(DAY, 15, 15)).await.unwrap();
    assert_eq!(swept.no_shows, 0);
    let refused = service::mark_no_show(t.state(), booking.id, Actor::staff(1), dubai(DAY, 15, 15))
        .await;

    match refused {
        Err(Refusal::Transition(error)) => {
            assert!(error.to_string().contains("open conflict"), "{error}");
        }
        other => panic!("the machine refuses: {other:?}"),
    }
    let same = t.reload(booking.id).await;
    assert_eq!(same.status, BookingStatus::Booked);
    assert!(same.conflict_block);
    assert!(t.events.no_show.all().is_empty());
}

/// The same through the staff route and the real clock: of two bookings whose
/// grace has passed, the one in conflict is refused and the other is marked;
/// the sweep marks a third and leaves the conflicted one.
#[renox::test]
async fn the_no_show_route_and_the_sweep_skip_the_conflicted_booking() {
    let t = boot().await;
    let s = scene(&t).await;
    let desk = t.front_desk().await;
    let made = dubai(PAST, 8, 0);
    let conflicted = book_at(&t, &s, &s.a, PAST, 11, 0, made).await;
    let control = book_at(&t, &s, &s.b, PAST, 11, 0, made).await;
    let swept = book_at(&t, &s, &s.b, PAST, 12, 0, made).await;
    let (_, flagged) = conflict::add_block(
        t.state(),
        AvailabilityBlock::manual(s.a.id, dubai(PAST, 10, 0), dubai(PAST, 13, 0), "sick"),
        made,
    )
    .await
    .unwrap();
    assert_eq!(flagged.len(), 1);

    t.app.acting_as(&desk);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/mark-no-show", conflicted.id),
            &[("go", "1")],
        )
        .await
        .assert_status(422);
    assert_eq!(t.reload(conflicted.id).await.status, BookingStatus::Booked);
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/mark-no-show", control.id),
            &[("go", "1")],
        )
        .await
        .assert_status(303);
    assert_eq!(t.reload(control.id).await.status, BookingStatus::NoShow);

    let marked = rules::no_shows(t.state(), now()).await.unwrap();

    assert_eq!(marked, 1);
    assert_eq!(t.reload(swept.id).await.status, BookingStatus::NoShow);
    assert_eq!(t.reload(conflicted.id).await.status, BookingStatus::Booked);
}

/// The conflict is cleared before 13:00 (the leave was cancelled): nothing is
/// escalated and the 13:00 reminder goes out normally.
#[renox::test]
async fn a_conflict_cleared_in_time_leaves_the_reminder_alone() {
    let t = boot().await;
    let s = scene(&t).await;
    let booking = book_at(&t, &s, &s.a, DAY, 15, 0, now()).await;
    t.state().emit(sick(605)).await.unwrap();
    assert!(t.reload(booking.id).await.conflict_block);
    t.state()
        .emit(LeaveCancelled {
            employee: EmployeeId(EMPLOYEE),
            request: 605,
            leave_type: "sick".to_owned(),
            from: date(DAY),
            to: date(DAY),
        })
        .await
        .unwrap();
    assert!(!t.reload(booking.id).await.has_conflict());

    let sent = rules::run(t.state(), dubai(DAY, 13, 0)).await.unwrap();

    assert_eq!(sent.escalations, 0);
    assert_eq!(sent.suppressed, 0);
    assert_eq!(sent.reminders, 1);
    let reminders = of_type(&t, "scheduling.booking_reminder");
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].subject, booking.subject());
    assert_eq!(reminders[0].recipients, [NotifyRecipient::Party { id: PATIENT }]);
    let body = Messages::body(&reminders[0]);
    assert!(body.contains(&booking.number), "{body}");
    assert!(body.contains(&format!("/scheduling/links/{}/cancel", booking.id)));
    assert!(of_type(&t, "scheduling.conflict_escalated").is_empty());
    assert!(of_type(&t, "scheduling.bookings_changed").is_empty());
}

/// Reminders go 24 hours and 2 hours before, once each, and only for a
/// `booked` or `confirmed` booking.
#[renox::test]
async fn reminders_go_out_once_at_each_time() {
    let t = boot().await;
    let s = scene(&t).await;
    let other = service::book(
        t.state(),
        NewBooking::new(OTHER_PATIENT, s.service.id, &[s.b.id], dubai(DAY, 15, 0)),
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();
    service::cancel(
        t.state(),
        other.id,
        service::CancelRequest::new("other"),
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();
    let booking = book_at(&t, &s, &s.a, DAY, 15, 0, now()).await;
    let day_before = "2030-03-11";

    let first = rules::run(t.state(), dubai(day_before, 15, 0)).await.unwrap();
    let repeat = rules::run(t.state(), dubai(day_before, 15, 5)).await.unwrap();
    let second = rules::run(t.state(), dubai(DAY, 13, 0)).await.unwrap();
    let third = rules::run(t.state(), dubai(DAY, 13, 10)).await.unwrap();

    assert_eq!(first.reminders, 1);
    assert_eq!(repeat.reminders, 0);
    assert_eq!(second.reminders, 1);
    assert_eq!(third.reminders, 0);
    let reminders = of_type(&t, "scheduling.booking_reminder");
    assert_eq!(reminders.len(), 2);
    assert!(reminders.iter().all(|r| r.subject == booking.subject()));
}
