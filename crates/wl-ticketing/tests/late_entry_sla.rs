//! BL-KRN-04 (#28) point 5 against the real ticketing: a ticket entered late
//! has its SLA counted from the business time (the time it was reported),
//! not from the time it was entered. The kernel records the business time of
//! the late entry; the ticket is then opened and assigned at that time
//! through `Tickets::open_at` and the real first assignment, whose due times
//! come from the policy and the SLA calendar.
//!
//! The SLA calendar is a flat 24-hour one so the numbers do not depend on the
//! clock of the machine; the working-hours arithmetic has its own tests in
//! `sla.rs` and `reassign.rs`.

mod support;

use renox::chrono::Duration;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::late_entries::{self, Entered, EntrySpec};
use wl_kernel::machines::BoxFuture;
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_kernel::{LateEntryDef, NewRecord, Subject};
use wl_ticketing::seed::STANDARD_POLICY;
use wl_ticketing::{Category, SlaCalendar, SlaPolicy, Tickets, WlTicketing, provide_sla_calendar};

use support::{CLIENT, boot_app};

const TICKET: &str = "ticketing.ticket";

/// Working time that is plain time.
struct Flat;

impl SlaCalendar for Flat {
    fn add_working(&self, from: DateTime, minutes: i64) -> DateTime {
        from + Duration::minutes(minutes)
    }

    fn working_minutes_between(&self, from: DateTime, to: DateTime) -> i64 {
        (to - from).num_minutes().max(0)
    }
}

/// The ticket row is opened by `Tickets` after the entry is recorded, so the
/// late-entry creator has nothing to write.
fn nothing_to_write<'a>(
    _tx: &'a mut renox::db::Transaction,
    _new: &'a NewRecord,
) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async { Ok(0) })
}

struct LateTickets;

impl Module for LateTickets {
    fn name(&self) -> &'static str {
        "test-late-tickets"
    }

    fn register(&self, app: &mut Registry) {
        LateEntryDef::of(TICKET, nothing_to_write)
            .business_time("reported_at")
            .register();
        provide_sla_calendar(app, Flat);
    }
}

/// A ticket reported 3 hours ago and entered now, response SLA 4 hours: the
/// response is due 1 hour from now (4 hours after it was reported), not 4
/// hours from now, and it is overdue from the second after that.
#[renox::test]
async fn krn04_the_business_time_decides_the_sla_of_a_real_ticket() {
    let t = boot_app(CLIENT, WlTicketing::new(), |app| app.module(LateTickets)).await;
    let anna = t.user("Anna", &[]).await;
    let imran = t.technician("Imran").await;

    // The response target of the category's priority is 4 hours.
    let category = Category::by_key(t.db(), "general").await.unwrap().unwrap();
    let policy = SlaPolicy::by_key(t.db(), STANDARD_POLICY)
        .await
        .unwrap()
        .unwrap();
    let mut target = policy
        .target(t.db(), category.default_priority)
        .await
        .unwrap()
        .unwrap();
    target.response_minutes = 240;
    target.save(t.db()).await.unwrap();

    let reported = now() - Duration::hours(3);
    let entered = late_entries::enter(
        t.state(),
        EntrySpec::new(TICKET, anna.id, reported).reason("The phone line was down"),
    )
    .await
    .unwrap();
    let Entered::Recorded { entry, .. } = entered else {
        panic!("expected a flagged late entry");
    };
    assert_eq!(entry.occurred_at, reported, "the business time is kept");
    assert!(entry.entered_at > entry.occurred_at);

    // The ticket is opened and assigned at the business time of the entry.
    let request = OpenTicket::new(Subject::new("property.unit", 12), "general", "No dial tone")
        .assign_to(Assignee::user(imran.id));
    let opened = Tickets::open_at(t.state(), request, entry.occurred_at)
        .await
        .unwrap();
    let ticket = t.ticket(opened.id.0).await;

    assert_eq!(ticket.sla_started_at, Some(reported));
    let due = ticket.response_due.expect("the response is due");
    assert_eq!(due, reported + Duration::hours(4));
    assert!(
        due < entry.entered_at + Duration::hours(4),
        "not 4 h after the entry"
    );
    let calendar = wl_ticketing::sla_calendar(t.state());
    assert_eq!(calendar.working_minutes_between(reported, due), 240);
    // Overdue is "after the due time": not at it, yes one second later.
    let overdue = |at: DateTime| at > due;
    assert!(!overdue(due));
    assert!(overdue(due + Duration::seconds(1)));
    assert!(now() < due, "one hour is left from now");
}
