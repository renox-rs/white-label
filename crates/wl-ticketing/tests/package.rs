//! The module as an app boots it: the acceptance checks AC1, AC3 and AC10
//! of #146 (the opening of a ticket and the seeded configuration).

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::tickets::{OpenTicket, TicketChannel, task_creator};
use wl_ticketing::{
    Category, HoldReason, Priority, ReassignReasons, SlaPolicy, Ticket, TicketStatus, WlTicketing,
    permissions as perm, seed,
};

use support::{boot, boot_with};

/// AC1: `/tickets` answers 200 with `tickets.view` and 403 without it.
#[renox::test]
async fn the_ticket_list_needs_the_view_permission() {
    let t = boot().await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    let nobody = t.user("Nora", &[]).await;
    t.open("general", None).await;

    t.app.acting_as(&viewer);
    t.app
        .get("/tickets")
        .await
        .assert_ok()
        .assert_see("TKT-2026-0001");
    t.app.acting_as(&nobody);
    t.app.get("/tickets").await.assert_forbidden();
}

#[renox::test]
async fn the_models_match_the_schema_and_every_view_compiles() {
    let t = boot().await;
    t.app.assert_models_match_schema().await;
    t.app.assert_views_compile();
}

/// AC3: a module that uses only `wl_kernel` opens a ticket on a unit; the
/// numbers run on, the ticket is `open` with the priority of its category.
#[renox::test]
async fn another_module_opens_tickets_through_the_task_creator() {
    let t = boot().await;
    let unit = Subject::new("property.unit", 12);
    let creator = task_creator(t.state());
    let first = creator
        .open(
            t.state(),
            OpenTicket::new(unit.clone(), "general", "Leaking tap"),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let second = creator
        .open(t.state(), OpenTicket::new(unit, "general", "Broken door"))
        .await
        .unwrap()
        .expect("ticketing is on");
    let year = wl_kernel::calendar::calendar(t.state())
        .today()
        .format("%Y")
        .to_string();
    assert_eq!(first.number, format!("TKT-{year}-0001"));
    assert_eq!(second.number, format!("TKT-{year}-0002"));

    let ticket = t.ticket(first.id.0).await;
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.priority, Priority::Normal);
    assert_eq!(ticket.channel, TicketChannel::System);
    assert_eq!(ticket.subject(), Subject::new("property.unit", 12));
}

/// AC3: an unknown category is refused, a ticket is born `open`, and its
/// status is saved only by a transition.
#[renox::test]
async fn an_unknown_category_is_refused_and_the_status_moves_only_by_transition() {
    let t = boot().await;
    let unit = Subject::new("property.unit", 12);
    let refused = task_creator(t.state())
        .open(t.state(), OpenTicket::new(unit, "no_such_category", "x"))
        .await;
    assert!(matches!(
        refused,
        Err(Error::Status(code, _)) if code == StatusCode::UNPROCESSABLE_ENTITY
    ));
    assert_eq!(Ticket::query().count(t.db()).await.unwrap(), 0);

    let mut ticket = t.open("general", None).await;
    ticket.status = TicketStatus::Closed;
    assert!(ticket.save(t.db()).await.is_err());
    assert_eq!(t.reload(&ticket).await.status, TicketStatus::Open);

    let mut born = Ticket {
        number: "TKT-2026-0099".into(),
        subject_type: "property.unit".into(),
        subject_id: 12,
        category_id: ticket.category_id,
        title: "x".into(),
        status: TicketStatus::InProgress,
        ..Ticket::default()
    };
    assert!(born.insert(t.db()).await.is_err());
}

/// AC10: seeding twice leaves one `standard` policy with the §7 targets,
/// the four hold reasons with their pause flags and the `general` category,
/// and keeps what a client changed.
#[renox::test]
async fn seeding_twice_keeps_one_set_of_defaults_and_the_clients_edits() {
    let t = boot().await;
    let again = seed::install(t.db()).await.unwrap();
    assert_eq!(again.total(), 0);

    assert_eq!(SlaPolicy::query().count(t.db()).await.unwrap(), 1);
    let standard = SlaPolicy::by_key(t.db(), "standard")
        .await
        .unwrap()
        .expect("the standard policy");
    // Working minutes: an SLA day is 9 working hours.
    let targets: Vec<_> = standard
        .targets(t.db())
        .await
        .unwrap()
        .into_iter()
        .map(|t| (t.priority, t.response_minutes, t.resolution_minutes))
        .collect();
    for expected in [
        (Priority::Urgent, 120, 24 * 60),
        (Priority::High, 8 * 60, 27 * 60),
        (Priority::Normal, 9 * 60, 63 * 60),
        (Priority::Low, 27 * 60, 126 * 60),
    ] {
        assert!(targets.contains(&expected), "{expected:?} in {targets:?}");
    }
    assert_eq!(targets.len(), 4);

    let reasons: Vec<_> = HoldReason::ordered(t.db())
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.key, r.pauses_sla))
        .collect();
    assert_eq!(reasons.len(), 4);
    for expected in [
        ("parts", true),
        ("access", true),
        ("waiting_on_requester", true),
        ("approval", false),
    ] {
        assert!(reasons.contains(&(expected.0.to_owned(), expected.1)));
    }
    assert_eq!(Category::query().count(t.db()).await.unwrap(), 2);

    // A client's edits stay.
    let mut approval = HoldReason::by_key(t.db(), "approval")
        .await
        .unwrap()
        .unwrap();
    approval.pauses_sla = true;
    approval.save(t.db()).await.unwrap();
    let mut general = Category::by_key(t.db(), "general").await.unwrap().unwrap();
    general.default_priority = Priority::High;
    general.save(t.db()).await.unwrap();
    let mut urgent = standard
        .target(t.db(), Priority::Urgent)
        .await
        .unwrap()
        .unwrap();
    urgent.response_minutes = 60;
    urgent.save(t.db()).await.unwrap();

    assert_eq!(seed::install(t.db()).await.unwrap().total(), 0);
    let approval = HoldReason::by_key(t.db(), "approval")
        .await
        .unwrap()
        .unwrap();
    assert!(approval.pauses_sla);
    let general = Category::by_key(t.db(), "general").await.unwrap().unwrap();
    assert_eq!(general.default_priority, Priority::High);
    let urgent = standard
        .target(t.db(), Priority::Urgent)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(urgent.response_minutes, 60);
    assert_eq!(SlaPolicy::query().count(t.db()).await.unwrap(), 1);
}

/// AC10: the five default reassignment reasons are provided, and a vertical
/// adds its own.
#[renox::test]
async fn the_reassignment_reasons_are_the_defaults_plus_the_verticals() {
    let t = boot().await;
    let reasons = t
        .state()
        .provided::<ReassignReasons>()
        .expect("the module provides its reasons");
    let codes: Vec<_> = reasons.all().iter().map(|r| r.code.as_str()).collect();
    assert_eq!(
        codes,
        [
            "vendor_no_show",
            "contract_ended",
            "vendor_document_expired",
            "workload",
            "other"
        ]
    );

    let vertical = boot_with(
        WlTicketing::new()
            .reassign_reason("tenant_request", "Tenant asked for someone else")
            .reassign_reason_with_text("escalated", "Escalated"),
    )
    .await;
    let reasons = vertical.state().provided::<ReassignReasons>().unwrap();
    assert_eq!(reasons.all().len(), 7);
    assert_eq!(reasons.check("tenant_request", ""), Ok(()));
    assert!(reasons.check("escalated", "").is_err());
}
