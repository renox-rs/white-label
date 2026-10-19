//! Staff tasks other modules open through the kernel's `TaskCreator`: the
//! acceptance check AC6 of #56 (due time, source key, comments).

mod support;

use wl_kernel::Subject;
use wl_kernel::tickets::{
    Assignee, OpenTicket, TASK_CATEGORY, TaskComment, TicketId, task_creator,
};
use wl_ticketing::{Comment, CommentVisibility, Priority, TicketStatus, permissions as perm};

use support::{boot, dubai};

const KEY: &str = "notify.give_up:ticketing.ticket:42:party:7";

/// AC6: `open` with an assignee, a due time and a source key gives an
/// `assigned` ticket in the `task` category with those values; `find_open`
/// finds it by its key until it is closed; `comment` adds an internal
/// comment, and the ticket page shows it with the due time.
#[renox::test]
async fn a_task_is_opened_found_commented_and_shown() {
    let t = boot().await;
    let holder = t.coordinator().await;
    let creator = task_creator(t.state());
    let due = dubai("2026-11-06", 10, 0);

    let opened = creator
        .open(
            t.state(),
            OpenTicket::new(
                Subject::new("ticketing.ticket", 42),
                TASK_CATEGORY,
                "Could not reach Sara M. about TKT-2026-0042",
            )
            .description("Channels tried:\n- Email s***@example.com: 550 mailbox unknown")
            .assign_to(Assignee::user(holder.id))
            .due_at(due)
            .source_key(KEY),
        )
        .await
        .unwrap()
        .expect("ticketing is on");

    let ticket = t.ticket(opened.id.0).await;
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.assignee(), Some(Assignee::user(holder.id)));
    assert_eq!(ticket.due_at, Some(due));
    assert_eq!(ticket.source_key.as_deref(), Some(KEY));
    assert_eq!(ticket.priority, Priority::Normal);
    assert_eq!(ticket.requester(), None);
    assert_eq!(t.history(&ticket).await.len(), 1, "the first assignment");

    let found = creator.find_open(t.state(), KEY).await.unwrap();
    assert_eq!(found, Some(opened.clone()));
    assert_eq!(
        creator
            .find_open(t.state(), "notify.give_up:other")
            .await
            .unwrap(),
        None
    );

    creator
        .comment(
            t.state(),
            opened.id,
            TaskComment::internal("Gave up again at 2026-11-06 09:00"),
        )
        .await
        .unwrap();
    let comments = Comment::of_ticket(t.db(), ticket.id).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].visibility, CommentVisibility::Internal);
    assert_eq!(comments[0].author_user_id, None);

    // Staff see the internal comment with the due time; a viewer who only
    // reads tickets does not see the comment.
    let path = format!("/tickets/{}", ticket.id);
    t.app.acting_as(&holder);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("06 Nov 2026 10:00")
        .assert_see("Gave up again at 2026-11-06 09:00");
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("06 Nov 2026 10:00")
        .assert_dont_see("Gave up again");

    // Closed (here: cancelled), it is no longer found.
    t.run(
        &holder,
        &ticket,
        "cancel",
        "2026-11-03 10:00",
        &[("reason", "Reached by phone")],
    )
    .await
    .assert_ok();
    assert_eq!(creator.find_open(t.state(), KEY).await.unwrap(), None);
}

/// AC6 of #49: `complete` closes an assigned task ticket with the note and
/// emits `TicketClosed`; a ticket that is closed already, or unknown, is
/// left alone and answers `false`; a blank note is refused.
#[renox::test]
async fn a_task_is_completed_by_its_source() {
    let t = boot().await;
    let holder = t.coordinator().await;
    let creator = task_creator(t.state());
    let key = "documents.renewal:hr.employee:7:work_permit";
    let opened = creator
        .open(
            t.state(),
            OpenTicket::new(
                Subject::new("hr.employee", 7),
                TASK_CATEGORY,
                "Renew Work permit of Ravi S.",
            )
            .assign_to(Assignee::user(holder.id))
            .source_key(key),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    assert_eq!(t.ticket(opened.id.0).await.status, TicketStatus::Assigned);

    let refused = creator.complete(t.state(), opened.id, "  ").await;
    assert!(refused.is_err(), "a blank note is refused");
    assert_eq!(t.ticket(opened.id.0).await.status, TicketStatus::Assigned);
    assert!(t.closed.all().is_empty());

    let note = "Renewed: Work permit, valid until 5 Dec 2027";
    assert!(creator.complete(t.state(), opened.id, note).await.unwrap());

    let ticket = t.ticket(opened.id.0).await;
    assert_eq!(ticket.status, TicketStatus::Closed);
    let closed = t.closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].ticket, opened.id);
    assert_eq!(creator.find_open(t.state(), key).await.unwrap(), None);

    // Closed already: nothing happens again.
    assert!(!creator.complete(t.state(), opened.id, note).await.unwrap());
    assert_eq!(t.closed.all().len(), 1);
    // An unknown ticket.
    assert!(
        !creator
            .complete(t.state(), TicketId(9999), note)
            .await
            .unwrap()
    );
}

/// A task without an assignee stays `open`; a blank source key is no key.
#[renox::test]
async fn a_task_without_a_holder_stays_open_and_unassigned() {
    let t = boot().await;
    let creator = task_creator(t.state());
    let opened = creator
        .open(
            t.state(),
            OpenTicket::new(Subject::new("ticketing.ticket", 1), TASK_CATEGORY, "x")
                .source_key("  "),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let ticket = t.ticket(opened.id.0).await;
    assert_eq!(ticket.status, TicketStatus::Open);
    assert_eq!(ticket.assignee(), None);
    assert_eq!(ticket.source_key, None);
    assert_eq!(ticket.due_at, None);
}
