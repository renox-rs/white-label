//! BL-TKT-13 (#473): a ticket entered late keeps the time it was reported:
//! flagged late entry with its reason, SLA from the reported time, *Overdue*
//! after the due time, listed in the next digest; a time within M minutes is
//! not flagged, a future time is refused, a portal ticket is never flagged.

mod support;

use renox::chrono::Duration;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::digest::{ItemKind, by_manager, gather};
use wl_kernel::tickets::{Assignee, OpenTicket, TicketChannel};
use wl_ticketing::late::{Reported, open_reported};
use wl_ticketing::seed::STANDARD_POLICY;
use wl_ticketing::{
    Category, Priority, SlaCalendar, SlaPolicy, Ticket, WlTicketing, provide_sla_calendar,
};

use support::{Booted, CLIENT, boot_app, dubai};

/// Working time that is plain time, so the due times do not depend on the
/// working hours.
struct Flat;

impl SlaCalendar for Flat {
    fn add_working(&self, from: DateTime, minutes: i64) -> DateTime {
        from + Duration::minutes(minutes)
    }

    fn working_minutes_between(&self, from: DateTime, to: DateTime) -> i64 {
        (to - from).num_minutes().max(0)
    }
}

struct FlatSla;

impl Module for FlatSla {
    fn name(&self) -> &'static str {
        "test-flat-sla"
    }

    fn register(&self, app: &mut Registry) {
        provide_sla_calendar(app, Flat);
    }
}

/// Booted with the flat calendar and the category "AC not cooling" whose
/// response target is 4 hours.
async fn boot() -> Booted {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app.module(FlatSla)).await;
    let policy = SlaPolicy::by_key(t.db(), STANDARD_POLICY)
        .await
        .unwrap()
        .unwrap();
    let mut target = policy
        .target(t.db(), Priority::Normal)
        .await
        .unwrap()
        .unwrap();
    target.response_minutes = 240;
    target.save(t.db()).await.unwrap();
    Category::create(
        t.db(),
        Category {
            key: "ac_not_cooling".into(),
            name_en: "AC not cooling".into(),
            default_priority: Priority::Normal,
            sla_policy_id: Some(policy.id),
            active: true,
            ..Category::default()
        },
    )
    .await
    .unwrap();
    t
}

fn request(imran: &User) -> OpenTicket {
    OpenTicket::new(
        Subject::new("property.unit", 12),
        "ac_not_cooling",
        "AC not cooling",
    )
    .channel(TicketChannel::Staff)
    .assign_to(Assignee::user(imran.id))
}

/// Opens `request` as `by` at the clock `entered`, reported at `reported`.
async fn enter(
    t: &Booted,
    by: &User,
    request: OpenTicket,
    entered: DateTime,
    reported: Reported,
) -> Result<Ticket> {
    t.travel_to(entered);
    let opened = t
        .app
        .at_travelled_time(open_reported(t.state(), by.id, request, reported))
        .await;
    t.app.travel_back();
    let opened = opened?;
    Ok(t.ticket(opened.id.0).await)
}

/// Done when 1 and 4: entered at 13:00, reported at 10:00 for a power cut:
/// flagged, response due 14:00, *Overdue* from 14:00:01, and in the next
/// digest of the manager.
#[renox::test]
async fn a_ticket_entered_late_runs_its_sla_from_the_reported_time() {
    let t = boot().await;
    let carla = t.coordinator().await;
    let imran = t.technician("Imran").await;
    let manager = t
        .user("Mona", &[wl_kernel::permissions::LATE_ENTRIES_APPROVE])
        .await;

    let ticket = enter(
        &t,
        &carla,
        request(&imran),
        dubai("2026-11-03", 13, 0),
        Reported::at(dubai("2026-11-03", 10, 0)).reason("power cut"),
    )
    .await
    .expect("the late ticket opens");
    assert!(ticket.late_entry, "flagged late entry");
    assert_eq!(ticket.late_reason.as_deref(), Some("power cut"));
    assert_eq!(ticket.reported_at, Some(dubai("2026-11-03", 10, 0)));
    assert_eq!(ticket.sla_started_at, Some(dubai("2026-11-03", 10, 0)));
    let due = dubai("2026-11-03", 14, 0);
    assert_eq!(ticket.response_due, Some(due));
    assert!(!ticket.is_overdue(due));
    assert!(ticket.is_overdue(due + Duration::seconds(1)));

    // The next digest lists it for the manager.
    let groups = gather(t.db(), dubai("2026-11-03", 0, 0), dubai("2026-11-04", 0, 0))
        .await
        .unwrap();
    let told = by_manager(t.db(), groups).await.unwrap();
    let sections = told.get(&manager.id).expect("the manager is told");
    assert!(sections.iter().flat_map(|s| &s.items).any(|item| {
        item.kind == ItemKind::Late
            && item.record == Subject::new("ticketing.ticket", ticket.id)
            && item.reason == "power cut"
    }));
}

/// Done when 2: entered at 10:05 (within M minutes): not flagged.
#[renox::test]
async fn a_ticket_entered_within_m_minutes_is_not_flagged() {
    let t = boot().await;
    let carla = t.coordinator().await;
    let imran = t.technician("Imran").await;
    let ticket = enter(
        &t,
        &carla,
        request(&imran),
        dubai("2026-11-03", 10, 5),
        Reported::at(dubai("2026-11-03", 10, 0)),
    )
    .await
    .expect("the ticket opens");
    assert!(!ticket.late_entry);
    assert_eq!(ticket.late_entry_id, None);
    assert_eq!(ticket.response_due, Some(dubai("2026-11-03", 14, 0)));
}

/// A late ticket without a reason is refused and nothing is stored.
#[renox::test]
async fn a_late_ticket_needs_a_reason() {
    let t = boot().await;
    let carla = t.coordinator().await;
    let imran = t.technician("Imran").await;
    let refused = enter(
        &t,
        &carla,
        request(&imran),
        dubai("2026-11-03", 13, 0),
        Reported::at(dubai("2026-11-03", 10, 0)),
    )
    .await;
    assert!(refused.is_err());
    assert_eq!(Ticket::query().count(t.db()).await.unwrap(), 0);
}

/// Done when 3: a *Reported at* in the future is refused.
#[renox::test]
async fn a_reported_time_in_the_future_is_refused() {
    let t = boot().await;
    let carla = t.coordinator().await;
    let imran = t.technician("Imran").await;
    let refused = enter(
        &t,
        &carla,
        request(&imran),
        dubai("2026-11-03", 10, 0),
        Reported::at(dubai("2026-11-03", 11, 0)),
    )
    .await;
    assert!(refused.is_err());
    assert_eq!(Ticket::query().count(t.db()).await.unwrap(), 0);
}

/// Done when 5: a portal ticket is never flagged late.
#[renox::test]
async fn a_portal_ticket_is_never_flagged() {
    let t = boot().await;
    let sara = t.user("Sara", &[]).await;
    let imran = t.technician("Imran").await;
    let ticket = enter(
        &t,
        &sara,
        request(&imran)
            .channel(TicketChannel::Portal)
            .requester_user(sara.id),
        dubai("2026-11-03", 13, 0),
        Reported::at(dubai("2026-11-03", 10, 0)),
    )
    .await
    .expect("the portal ticket opens");
    assert!(!ticket.late_entry);
    assert_eq!(ticket.late_entry_id, None);
    assert_ne!(ticket.sla_started_at, Some(dubai("2026-11-03", 10, 0)));
}

/// The staff form asks for the reason of a late ticket and shows the flag.
#[renox::test]
async fn the_staff_form_takes_the_reported_time_and_reason() {
    let t = boot().await;
    let carla = t.coordinator().await;
    t.app.acting_as(&carla);
    let three_hours_ago = (renox::db::now() - Duration::hours(3))
        .with_timezone(&renox::chrono::FixedOffset::east_opt(4 * 3600).unwrap())
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    let form = [
        ("subject_type", "property.unit"),
        ("subject_id", "12"),
        ("category", "ac_not_cooling"),
        ("title", "AC not cooling"),
        ("reported_at", three_hours_ago.as_str()),
    ];
    t.app
        .post("/tickets", &form)
        .await
        .assert_invalid("late_reason");
    let mut with_reason = form.to_vec();
    with_reason.push(("late_reason", "power cut"));
    t.app.post("/tickets", &with_reason).await;
    let ticket = Ticket::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a ticket");
    assert!(ticket.late_entry);
    t.app
        .get(&format!("/tickets/{}", ticket.id))
        .await
        .assert_ok()
        .assert_see("Late entry")
        .assert_see("power cut");
}
