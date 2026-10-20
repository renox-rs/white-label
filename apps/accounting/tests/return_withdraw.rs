//! #340 (BL-ACC-16): a client approver withdraws the approval of a return
//! before it is filed (04 §7.1 *Return* `withdraw_approval`, `DOC.SIG.01`).
//! The real wl-ticketing holds the linked calendar task and the preparer's
//! task, so the Done-when points are proved through the real routes and the
//! real transitions: the return goes back to `preparing`, the signature is
//! kept and marked withdrawn, the preparer and the reviewer are told, the
//! linked task stays where it is, and the preparer's task says what to do
//! with it, also after `ACC.RUL.04` makes it overdue.

mod support;

use accounting::return_effects::{WITHDRAWN_NOTICE, withdrawn_key};
use accounting::{Return, ReturnSignature, ReturnStatus};
use renox::prelude::*;
use wl_kernel::deadlines::DeadlineTaskStatus;
use wl_kernel::documents::dubai_date;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::task_creator;
use wl_ticketing::{Comment, Ticket};

use support::{Booted, Crew, boot_real, d};

/// An `approved` VAT return of a fresh active engagement, its linked task
/// (due 2026-11-28) `in_review`.
async fn approved(t: &Booted) -> (Return, Crew, wl_kernel::deadlines::DeadlineTaskId) {
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let engagement = t.active_engagement(&client).await;
    let task = t.vat_task(&engagement, d(11, 28)).await;
    t.task_in_review(&crew.reviewer, task).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    assert_eq!(ret.task_id, Some(task.0), "linked to the calendar task");
    let ret = t.return_at(&ret, &crew, ReturnStatus::Approved).await;
    (ret, crew, task)
}

/// The preparer's task for the withdrawn approval of `ret`.
async fn preparer_task(t: &Booted, ret: &Return) -> Ticket {
    let key = withdrawn_key(ret.id);
    Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the preparer has a task")
}

/// Done when 1: the client approver withdraws with "sales figure wrong": the
/// return is `preparing`, the signature is kept and marked withdrawn, the
/// preparer and the reviewer are told, the linked task stays `in_review`, and
/// the preparer has a task "Client withdrew approval" due the next working
/// day that names the linked task and its state.
#[renox::test]
async fn the_client_approver_withdraws_the_approval() {
    let t = boot_real().await;
    let (ret, crew, task) = approved(&t).await;
    assert_eq!(
        ReturnSignature::of_return(t.db(), ret.id)
            .await
            .unwrap()
            .len(),
        1
    );

    t.set_clock(10, 10, 0);
    t.portal_act(
        &crew.approver,
        &ret,
        "withdraw",
        &[("comment", "sales figure wrong")],
    )
    .await
    .assert_ok();

    assert_eq!(t.return_status(&ret).await, ReturnStatus::Preparing);
    let signatures = ReturnSignature::of_return(t.db(), ret.id).await.unwrap();
    assert_eq!(signatures.len(), 1, "the signature is kept");
    assert!(signatures[0].is_withdrawn());
    assert_eq!(
        signatures[0].withdrawn_reason.as_deref(),
        Some("sales figure wrong")
    );
    assert_eq!(signatures[0].withdrawn_by, Some(crew.approver.id));
    assert_eq!(signatures[0].withdrawn_instruction, None);
    assert_eq!(signatures[0].signer_name, "Omar");

    let told = t.notices.of_type(WITHDRAWN_NOTICE);
    assert_eq!(told.len(), 1);
    for user in [&crew.preparer, &crew.reviewer] {
        assert!(
            told[0]
                .recipients
                .contains(&NotifyRecipient::User { id: user.id }),
            "{} is told",
            user.name
        );
    }
    assert_eq!(told[0].data["reason"], "sales figure wrong");

    assert_eq!(
        t.task_now(task).await.status,
        DeadlineTaskStatus::InReview,
        "the linked task is not moved"
    );
    let todo = preparer_task(&t, &ret).await;
    assert_eq!(todo.title, "Client withdrew approval of RET-0001");
    assert_eq!(todo.assignee_user_id, Some(crew.preparer.id));
    assert_eq!(dubai_date(todo.due_at.expect("a due time")), d(11, 11));
    assert!(todo.description.contains("sales figure wrong"));
    assert!(
        todo.description
            .contains("VAT return, period ending 2026-10-31 (in_review)"),
        "{}",
        todo.description
    );
    assert!(
        todo.description.contains("send_back"),
        "{}",
        todo.description
    );
}

/// Done when 2: a reviewer withdrawing on the client's written instruction
/// must attach it; without it the withdrawal is refused and the return stays
/// `approved`. With it the return is `preparing` and the instruction is kept
/// on the withdrawn signature.
#[renox::test]
async fn a_reviewer_must_attach_the_clients_instruction() {
    let t = boot_real().await;
    let (ret, crew, _task) = approved(&t).await;

    t.ret_act(
        &crew.reviewer,
        &ret,
        "withdraw",
        &[("comment", "The client called"), ("instruction", "  ")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Approved);
    t.ret_act(
        &crew.reviewer,
        &ret,
        "withdraw",
        &[("instruction", "Email of 5 Nov")],
    )
    .await
    .assert_status(422);
    assert_eq!(
        t.return_status(&ret).await,
        ReturnStatus::Approved,
        "no reason"
    );

    t.ret_act(
        &crew.reviewer,
        &ret,
        "withdraw",
        &[
            ("comment", "The client called"),
            ("instruction", "Email of 5 Nov: please hold the return"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Preparing);
    let signatures = ReturnSignature::of_return(t.db(), ret.id).await.unwrap();
    assert_eq!(
        signatures[0].withdrawn_instruction.as_deref(),
        Some("Email of 5 Nov: please hold the return")
    );
    assert_eq!(signatures[0].withdrawn_by, Some(crew.reviewer.id));
}

/// A client user without the *approver* role cannot withdraw; neither can a
/// staff member without `returns.review`. Any *approver* of the client can,
/// not only the one who approved.
#[renox::test]
async fn any_client_approver_may_withdraw_and_nobody_else() {
    let t = boot_real().await;
    let (ret, crew, _task) = approved(&t).await;
    let client = ret.client();
    let finance = t
        .portal_user_with("Fatma", crew_person(&t).await, client, &["finance"])
        .await;
    t.portal_act(&finance, &ret, "withdraw", &[("comment", "no")])
        .await
        .assert_status(422);
    t.ret_act(
        &crew.preparer,
        &ret,
        "withdraw",
        &[("comment", "no"), ("instruction", "x")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Approved);

    let second = t
        .portal_user_with("Sara", crew_person(&t).await, client, &["approver"])
        .await;
    t.portal_act(&second, &ret, "withdraw", &[("comment", "my mistake")])
        .await
        .assert_ok();
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Preparing);
}

/// A person (party) for the extra portal users.
async fn crew_person(t: &Booted) -> wl_kernel::PartyId {
    t.party("Another person", wl_kernel::PartyKind::Person)
        .await
}

/// Done when 3: the linked task still `in_review` when its due date
/// 2026-11-28 passes after the withdrawal: `ACC.RUL.04` makes it `overdue` on
/// 2026-11-29 and the preparer's task now says to `start` it. Submitting the
/// return again closes the preparer's task and the withdrawn signature stays
/// in the history next to the new one after the next approval.
#[renox::test]
async fn an_overdue_linked_task_is_to_be_started_and_resubmitting_closes_the_task() {
    let t = boot_real().await;
    let (ret, crew, task) = approved(&t).await;
    t.set_clock(10, 10, 0);
    t.portal_act(
        &crew.approver,
        &ret,
        "withdraw",
        &[("comment", "sales figure wrong")],
    )
    .await
    .assert_ok();
    let todo = preparer_task(&t, &ret).await;

    t.run(wl_ticketing::deadlines::mark_overdue(t.state(), d(11, 29)))
        .await
        .unwrap();
    assert_eq!(t.task_now(task).await.status, DeadlineTaskStatus::Overdue);
    let comments = Comment::of_ticket(t.db(), todo.id).await.unwrap();
    assert!(
        comments
            .iter()
            .any(|c| c.text.contains("now overdue") && c.text.contains("start it")),
        "{:?}",
        comments.iter().map(|c| &c.text).collect::<Vec<_>>()
    );

    // The rework: submitted again, the preparer's task closes.
    let ret = t.return_now(&ret).await;
    t.return_at_again(&ret, &crew).await;
    let creator = task_creator(t.state());
    assert!(
        creator
            .find_open(t.state(), &withdrawn_key(ret.id))
            .await
            .unwrap()
            .is_none(),
        "the preparer's task is closed"
    );
    // Approved again: two signatures, the first one withdrawn.
    t.portal_act(&crew.approver, &ret, "approve", &[])
        .await
        .assert_ok();
    let signatures = ReturnSignature::of_return(t.db(), ret.id).await.unwrap();
    assert_eq!(signatures.len(), 2);
    assert!(signatures[0].is_withdrawn());
    assert!(!signatures[1].is_withdrawn());
}

/// Done when 4 and the Must-not: a `filed` return offers no withdrawal (the
/// route refuses, the pages show no button) and its signature stays valid.
#[renox::test]
async fn a_filed_return_offers_no_withdrawal() {
    let t = boot_real().await;
    let (ret, crew, _task) = approved(&t).await;
    t.ret_act(
        &crew.partner,
        &ret,
        "file",
        &[("acknowledgement", "FTA-2026-1188")],
    )
    .await
    .assert_ok();
    let filed = t.return_now(&ret).await;
    assert_eq!(filed.status, ReturnStatus::Filed);
    assert_eq!(filed.acknowledgement.as_deref(), Some("FTA-2026-1188"));

    t.portal_act(&crew.approver, &ret, "withdraw", &[("comment", "too late")])
        .await
        .assert_status(422);
    t.ret_act(
        &crew.reviewer,
        &ret,
        "withdraw",
        &[("comment", "too late"), ("instruction", "x")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.return_status(&ret).await, ReturnStatus::Filed);

    t.get(&crew.reviewer, &format!("/accounting/returns/{}", ret.id))
        .await
        .assert_ok()
        .assert_dont_see("Withdraw approval");
    t.get(
        &crew.approver,
        &format!("/accounting/portal/returns/{}", ret.id),
    )
    .await
    .assert_ok()
    .assert_dont_see("Withdraw approval");
    let signatures = ReturnSignature::of_return(t.db(), ret.id).await.unwrap();
    assert!(!signatures[0].is_withdrawn(), "the signature stays valid");
}

/// While `approved` the pages do offer the withdrawal.
#[renox::test]
async fn an_approved_return_offers_the_withdrawal() {
    let t = boot_real().await;
    let (ret, crew, _task) = approved(&t).await;
    t.get(&crew.reviewer, &format!("/accounting/returns/{}", ret.id))
        .await
        .assert_ok()
        .assert_see("Withdraw approval");
    t.get(
        &crew.approver,
        &format!("/accounting/portal/returns/{}", ret.id),
    )
    .await
    .assert_ok()
    .assert_see("Withdraw approval");
}
