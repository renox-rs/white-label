//! Acceptance checks of #329 (BL-ACC-05): a suspended engagement resumes
//! with the manager's note (refused without one); for *non-payment* the page
//! shows the client's overdue invoices; never while the client is blocked;
//! after a block only on the MLRO's approval. The deadline-notice follow-up
//! of the suspension is cancelled. The release of the tasks, the retainer and
//! the skipped request lists with the real modules are in `resume_real.rs`.

mod support;

use accounting::deadline_notices::{DEADLINE_REMINDER, DeadlineNotice, follow_up};
use accounting::engagement_machine::RESUME;
use accounting::{Engagement, EngagementStatus};
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_kernel::subscriptions::PauseCause;

use support::{Booted, boot, d};

/// The approval requests still pending about `engagement`.
async fn pending(t: &Booted, engagement: &Engagement) -> Vec<approvals::ApprovalRequest> {
    approvals::pending_for(t.db(), &engagement.subject())
        .await
        .unwrap()
}

/// DW2: suspended for *non-payment* with AED 6,000 still overdue: the page
/// shows the AED 6,000; `resume` without a note is refused, with one it
/// succeeds. The tasks are released, the retainer resumed, and the
/// suspension's deadline notice sends no more reminder.
#[renox::test]
async fn a_non_payment_suspension_shows_what_is_overdue_and_resumes_with_a_note() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.invoices
        .overdue(client.company, "INV-FLA-2026-0007", 600_000, d(10, 15));

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "non_payment")],
    )
    .await
    .assert_ok();
    let page = t
        .get(
            &manager,
            &format!("/accounting/engagements/{}", engagement.id),
        )
        .await;
    page.assert_ok()
        .assert_see("Overdue invoices")
        .assert_see("INV-FLA-2026-0007")
        .assert_see("AED 6,000.00");

    t.set_clock(4, 10, 0);
    t.act(&manager, &engagement, "resume", &[])
        .await
        .assert_status(422);
    t.act(&manager, &engagement, "resume", &[("note", "   ")])
        .await
        .assert_status(422);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);

    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "Payment plan agreed: AED 2,000 a month")],
    )
    .await
    .assert_ok();
    let resumed = t.reload(&engagement).await;
    assert_eq!(resumed.status, EngagementStatus::Active);
    assert_eq!(resumed.reason_code, None);
    assert_eq!(
        resumed.resume_note.as_deref(),
        Some("Payment plan agreed: AED 2,000 a month")
    );
    assert!(pending(&t, &engagement).await.is_empty(), "no approval");
    assert_eq!(t.tasks.releases(), [(engagement.subject(), d(11, 4))]);
    assert_eq!(
        t.subscriptions.resumes(),
        [(engagement.subject(), PauseCause::Agreement, d(11, 4))]
    );

    // The deadline notice of the suspension: cancelled, no reminder.
    let notices = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].cancelled_on, Some(d(11, 4)));
    let later = t.run(follow_up(t.state(), d(11, 9))).await.unwrap();
    assert!(later.reminded.is_empty() && later.escalated.is_empty());
    assert!(t.notices.of_type(DEADLINE_REMINDER).is_empty());
}

/// DW3 and DW4: suspended *on hold* by a block, `resume` is refused while
/// the party is blocked; once unblocked it becomes an approval request to
/// the MLRO and the engagement stays `suspended`; a rejection keeps it
/// `suspended`, the MLRO's approval makes it `active`.
#[renox::test]
async fn after_a_block_the_resume_waits_for_the_unblock_and_the_mlro() {
    let t = boot().await;
    let manager = t.manager().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.block(client.company, "non_payment").await;
    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));

    // DW3: still blocked.
    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "The client paid")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);
    assert!(pending(&t, &engagement).await.is_empty());

    // DW4: unblocked, the resume needs the MLRO.
    t.unblock(&mlro, client.company, "Arrears settled in full")
        .await;
    t.get(
        &manager,
        &format!("/accounting/engagements/{}", engagement.id),
    )
    .await
    .assert_ok()
    .assert_see("the resume needs the MLRO");
    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "The client paid")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);
    let requests = pending(&t, &engagement).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transition, RESUME);
    assert_eq!(requests[0].requester_id, manager.id);
    assert!(
        t.tasks.releases().is_empty(),
        "nothing is released before the yes"
    );

    // A rejection keeps it suspended.
    t.run(approvals::reject(
        t.state(),
        requests[0].id,
        &mlro,
        "Wait for the second instalment",
    ))
    .await
    .unwrap();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);

    // Asked again and approved: active.
    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "Second instalment received")],
    )
    .await
    .assert_ok();
    let requests = pending(&t, &engagement).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, ApprovalStatus::Pending);
    t.run(approvals::approve(
        t.state(),
        requests[0].id,
        &mlro,
        "Cleared",
    ))
    .await
    .unwrap();
    let resumed = t.reload(&engagement).await;
    assert_eq!(resumed.status, EngagementStatus::Active);
    assert_eq!(
        resumed.resume_note.as_deref(),
        Some("Second instalment received")
    );
    assert_eq!(t.tasks.releases().len(), 1, "released on the yes");
    assert_eq!(t.subscriptions.resumes().len(), 1);
}
