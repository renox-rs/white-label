//! BL-TKT-12 (#472): a ticket goes to its category's role in the subject's
//! scope, waits in the queue of every holder, and one holder takes it; a
//! category without a role, or a role nobody holds, leaves it `open`.

mod support;

use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::Assignee;
use wl_ticketing::role_queue::{self, TAKEN_FROM_QUEUE, UNROUTED};
use wl_ticketing::{Category, Priority, TicketStatus, permissions as perm};

use support::{Booted, boot};

/// The notice every holder of the role gets.
const ASSIGNED: &str = "ticketing.assigned";

/// A category `key` whose default assignee role is `role`, with the SLA
/// policy of the seeded general category.
async fn category(t: &Booted, key: &str, role: Option<&str>) {
    let general = Category::by_key(t.db(), "general")
        .await
        .unwrap()
        .expect("the general category");
    Category::create(
        t.db(),
        Category {
            key: key.to_owned(),
            name_en: key.replace('_', " "),
            default_priority: Priority::Normal,
            sla_policy_id: general.sla_policy_id,
            active: true,
            default_assignee_role: role.map(str::to_owned),
            ..Category::default()
        },
    )
    .await
    .unwrap();
}

/// A user holding `role` (which grants `grants`) everywhere.
async fn holder(t: &Booted, name: &str, role: &str, grants: &[&str]) -> User {
    t.user_in_role(name, role, grants).await
}

fn users(recipients: &[NotifyRecipient]) -> Vec<i64> {
    recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// Done when 1 and 2 (the mechanism): the ticket is assigned to the role,
/// both holders see it in their queue and are told; Omar takes it, it is
/// his with the reason *taken from queue*, it leaves Nadia's queue and the
/// SLA due times stay.
#[renox::test]
async fn a_role_ticket_waits_in_every_holders_queue_until_one_takes_it() {
    let t = boot().await;
    category(&t, "ac_repair", Some("hvac")).await;
    let nadia = holder(&t, "Nadia", "hvac", &[perm::VIEW]).await;
    let omar = User::register(t.db(), "Omar", "omar@example.com", "password123")
        .await
        .unwrap();
    omar.assign_role(t.db(), "hvac").await.unwrap();

    let ticket = t.open("ac_repair", None).await;
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.assignee(), None, "no user, no vendor");
    assert_eq!(ticket.assignee_role.as_deref(), Some("hvac"));
    assert_eq!(ticket.assignee_role_display.as_deref(), Some("Hvac"));
    assert!(
        ticket.sla_started_at.is_some(),
        "the SLA starts at the routing"
    );
    let response_due = ticket.response_due.expect("a response due time");

    for user in [&nadia, &omar] {
        let queue = role_queue::queue_of(t.db(), user.id).await.unwrap();
        assert_eq!(queue.iter().map(|q| q.id).collect::<Vec<_>>(), [ticket.id]);
    }
    let told = users(&t.notices.recipients(ASSIGNED));
    assert!(
        told.contains(&nadia.id) && told.contains(&omar.id),
        "{told:?}"
    );

    t.app.acting_as(&nadia);
    t.app
        .get("/tickets")
        .await
        .assert_ok()
        .assert_see(&ticket.number)
        .assert_see("Hvac");

    t.app.acting_as(&omar);
    t.app
        .post(&format!("/tickets/{}/take", ticket.id), &[])
        .await
        .assert_redirect(&format!("/tickets/{}", ticket.id));
    let taken = t.reload(&ticket).await;
    assert_eq!(taken.status, TicketStatus::Assigned);
    assert_eq!(taken.assignee(), Some(Assignee::user(omar.id)));
    assert_eq!(taken.assignee_role, None);
    assert_eq!(
        taken.response_due,
        Some(response_due),
        "the SLA is unchanged"
    );
    assert_eq!(taken.sla_started_at, ticket.sla_started_at);
    let history = t.history(&ticket).await;
    let last = history.last().expect("the take is in the history");
    assert_eq!(last.reason_code.as_deref(), Some(TAKEN_FROM_QUEUE));
    assert_eq!(last.assignee_user_id, Some(omar.id));
    assert!(
        role_queue::queue_of(t.db(), nadia.id)
            .await
            .unwrap()
            .is_empty(),
        "it left Nadia's queue"
    );
}

/// Only a holder of the role takes the ticket.
#[renox::test]
async fn someone_without_the_role_cannot_take_it() {
    let t = boot().await;
    category(&t, "ac_repair", Some("hvac")).await;
    let _nadia = holder(&t, "Nadia", "hvac", &[perm::VIEW]).await;
    let sam = t.technician("Sam").await;
    let ticket = t.open("ac_repair", None).await;
    t.app.acting_as(&sam);
    t.app
        .post(&format!("/tickets/{}/take", ticket.id), &[])
        .await
        .assert_not_found();
    assert_eq!(t.reload(&ticket).await.assignee(), None);
}

/// Done when 4: a category without a default role leaves the ticket `open`
/// for the coordinator.
#[renox::test]
async fn a_category_without_a_role_leaves_the_ticket_open() {
    let t = boot().await;
    let _nadia = holder(&t, "Nadia", "hvac", &[perm::VIEW]).await;
    let ticket = t.open("general", None).await;
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.assignee_role, None);
    assert!(ticket.sla_started_at.is_none());
}

/// Done when 5 (the mechanism): nobody holds the role, so the ticket stays
/// `open` and the company admin is told.
#[renox::test]
async fn nobody_in_the_role_leaves_it_open_and_tells_the_company_admin() {
    let t = boot().await;
    category(&t, "plumbing", Some("plumber")).await;
    let amira = holder(&t, "Amira", wl_kernel::COMPANY_ADMIN_ROLE, &[perm::VIEW]).await;
    let ticket = t.open("plumbing", None).await;
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.assignee_role, None);
    assert_eq!(users(&t.notices.recipients(UNROUTED)), [amira.id]);
}

/// Done when 6: a user without the role in the scope does not see the
/// ticket in any list, nor its page.
#[renox::test]
async fn a_user_without_the_role_does_not_see_the_ticket() {
    let t = boot().await;
    category(&t, "ac_repair", Some("hvac")).await;
    let _nadia = holder(&t, "Nadia", "hvac", &[perm::VIEW]).await;
    let sam = t.technician("Sam").await;
    let ticket = t.open("ac_repair", None).await;
    t.app.acting_as(&sam);
    t.app
        .get("/tickets")
        .await
        .assert_ok()
        .assert_dont_see(&ticket.number);
    t.app
        .get("/tickets?queue=mine")
        .await
        .assert_ok()
        .assert_dont_see(&ticket.number);
    t.app
        .get(&format!("/tickets/{}", ticket.id))
        .await
        .assert_not_found();
    assert!(
        role_queue::queue_of(t.db(), sam.id)
            .await
            .unwrap()
            .is_empty()
    );
}
