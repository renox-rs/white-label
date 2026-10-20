//! #341 (BL-ACC-17): the manager cancels a return the firm will not file
//! (04 §7.1 *Return* `cancel`). The real wl-ticketing holds the linked
//! calendar task: *not required* waives an `upcoming` one and lists a
//! started one for the manager; *client files itself* and *engagement ended*
//! hand it over to the client (completed with the evidence "handed over to
//! client"); the client is told who files it and when, except after a CDD
//! exit; the portal shows the deadline as the client's.

mod support;

use accounting::return_effects::{HANDED_OVER_NOTICE, left_key};
use accounting::{Return, ReturnStatus};
use renox::prelude::*;
use wl_kernel::deadlines::{DeadlineTaskId, DeadlineTaskStatus};
use wl_kernel::notify::{self, NotifyChannel, NotifyRecipient};
use wl_ticketing::Ticket;

use support::{Booted, Client, Crew, boot_real, d, register_portal_section};

/// A fresh active engagement of a client with a VAT task due 2026-11-28 and a
/// return for it in `status`.
async fn scenario(t: &Booted, status: ReturnStatus) -> (Client, Crew, Return, DeadlineTaskId) {
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let engagement = t.active_engagement(&client).await;
    let task = t.vat_task(&engagement, d(11, 28)).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    let ret = t.return_at(&ret, &crew, status).await;
    (client, crew, ret, task)
}

/// Done when 1: a VAT return in `client_approval`, cancelled by the manager
/// with *client files itself*: it is `cancelled`, the linked task is handed
/// over (completed with the evidence), the client approver is told it files
/// the return due 2026-11-28 itself, and the portal shows the deadline as
/// the client's.
#[renox::test]
async fn a_return_the_client_files_itself_is_handed_over_and_the_client_told() {
    register_portal_section();
    let t = boot_real().await;
    let (client, crew, ret, task) = scenario(&t, ReturnStatus::ClientApproval).await;

    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "client_files_itself")],
    )
    .await
    .assert_ok();

    let cancelled = t.return_now(&ret).await;
    assert_eq!(cancelled.status, ReturnStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("client_files_itself")
    );
    let handed = wl_ticketing::DeadlineTask::find(t.db(), task.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(handed.status, DeadlineTaskStatus::Completed);
    assert_eq!(handed.evidence.as_deref(), Some("handed over to client"));

    let told = t.notices.of_type(HANDED_OVER_NOTICE);
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].recipients,
        [NotifyRecipient::Party { id: client.ubo }],
        "the client approver"
    );
    let def = notify::notification_type(HANDED_OVER_NOTICE).expect("registered");
    let body = def
        .template_for(NotifyChannel::Email, "en")
        .expect("an English template")
        .body
        .clone();
    let text = notify::fill(&body, &told[0].data);
    assert!(text.contains("You file it yourselves"), "{text}");
    assert!(text.contains("2026-11-28"), "{text}");

    // The portal shows the deadline as the client's.
    let page = t
        .get(
            &crew.approver,
            &format!("/portal/{}/returns", client.company.0),
        )
        .await;
    page.assert_ok().assert_see("You file it by 2026-11-28");
    t.get(
        &crew.approver,
        &format!("/accounting/portal/returns/{}", ret.id),
    )
    .await
    .assert_ok()
    .assert_see("You file it by 2026-11-28");
}

/// Done when 2: *not required* and an `upcoming` task: the task is
/// `not_required`; the client is told nothing (nobody files it).
#[renox::test]
async fn a_return_not_required_waives_an_upcoming_task() {
    let t = boot_real().await;
    let (_client, crew, ret, task) = scenario(&t, ReturnStatus::Preparing).await;

    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "not_required")],
    )
    .await
    .assert_ok();
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Cancelled);
    let waived = t.task_now(task).await;
    assert_eq!(waived.status, DeadlineTaskStatus::NotRequired);
    assert_eq!(waived.closed_reason.as_deref(), Some("Return not required"));
    assert!(t.notices.of_type(HANDED_OVER_NOTICE).is_empty());
}

/// *Not required* with a started task: the task is not waived (04 keeps
/// *waive a started task* off); it is listed for the manager.
#[renox::test]
async fn a_started_task_of_a_return_not_required_is_listed_for_the_manager() {
    let t = boot_real().await;
    let (_client, crew, ret, task) = scenario(&t, ReturnStatus::InReview).await;
    t.task_act(&crew.reviewer, task, "start", &[])
        .await
        .assert_ok();

    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "not_required")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.task_now(task).await.status,
        DeadlineTaskStatus::InProgress,
        "not waived"
    );
    let key = left_key(ret.id);
    let listing = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the manager's task");
    assert_eq!(listing.assignee_user_id, Some(crew.reviewer.id));
    assert!(
        listing
            .description
            .contains("VAT return, period ending 2026-10-31 is in_progress"),
        "{}",
        listing.description
    );
}

/// *Engagement ended* hands the task over too, with no next occurrence, and
/// tells the client.
#[renox::test]
async fn a_return_of_an_ended_engagement_is_handed_over() {
    let t = boot_real().await;
    let (_client, crew, ret, task) = scenario(&t, ReturnStatus::Approved).await;
    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();
    let cancelled = t.return_now(&ret).await;
    assert_eq!(cancelled.status, ReturnStatus::Cancelled);
    assert_eq!(t.task_now(task).await.status, DeadlineTaskStatus::Completed);
    assert_eq!(t.notices.of_type(HANDED_OVER_NOTICE).len(), 1);
    // The approval's signature is kept: the return is never filed by the firm.
    let signatures = accounting::ReturnSignature::of_return(t.db(), ret.id)
        .await
        .unwrap();
    assert_eq!(signatures.len(), 1);
    assert!(!signatures[0].is_withdrawn());
}

/// After a CDD exit the client is told nothing automatically
/// (`ACC.ACL.14`): the MLRO's template tells it; the task is still handed
/// over.
#[renox::test]
async fn after_a_cdd_exit_the_client_is_told_nothing() {
    let t = boot_real().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t.active_engagement(&client).await;
    let task = t.vat_task(&engagement, d(11, 28)).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", "AML concern")],
    )
    .await
    .assert_ok();

    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();
    let cancelled = t.return_now(&ret).await;
    assert!(cancelled.client_messages_suppressed);
    assert_eq!(t.task_now(task).await.status, DeadlineTaskStatus::Completed);
    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}"
        );
    }
    assert!(t.notices.of_type(HANDED_OVER_NOTICE).is_empty());
}

/// Done when 3 and the Must-nots: a `filed` return offers no `cancel` (the
/// route refuses, the page shows no button), and a cancel needs a listed
/// reason and the manager.
#[renox::test]
async fn a_filed_return_offers_no_cancel_and_a_cancel_needs_a_reason() {
    let t = boot_real().await;
    let (_client, crew, ret, _task) = scenario(&t, ReturnStatus::Approved).await;

    // No reason, an unknown reason, and not the preparer.
    t.ret_act(&crew.reviewer, &ret, "cancel", &[])
        .await
        .assert_status(422);
    t.ret_act(&crew.reviewer, &ret, "cancel", &[("reason", "sanctions")])
        .await
        .assert_status(422);
    t.ret_act(
        &crew.preparer,
        &ret,
        "cancel",
        &[("reason", "not_required")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Approved);

    t.get(&crew.reviewer, &format!("/accounting/returns/{}", ret.id))
        .await
        .assert_ok()
        .assert_see("Cancel return");
    t.file_return(&crew.partner, &ret).await.assert_ok();
    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "not_required")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Filed);
    t.get(&crew.reviewer, &format!("/accounting/returns/{}", ret.id))
        .await
        .assert_ok()
        .assert_dont_see("Cancel return");
}
