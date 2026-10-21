//! Acceptance checks of #333 (BL-ACC-09): the MLRO exits a client through
//! CDD (or rejects the file of an existing client at a review) and the
//! partner gets one task to end the engagements with the neutral reason
//! *relationship ended*; the reason stays with the MLRO and deputy, no new
//! work is accepted, and nothing reaches the client by itself. With the
//! real wl-ticketing.

mod support;

use accounting::client_notices::ENDED_NOTICE;
use accounting::deadline_notices::DeadlineNotice;
use accounting::exits::{DUE_WORKING_DAYS, partner_key};
use accounting::{CddFile, CddStatus, EngagementStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;

use support::{Booted, Client, boot_real, d, dubai};

/// The reason only the MLRO sees.
const REASON: &str = "Adverse media: the owner is under investigation";

/// The tickets with the source key `key`.
async fn tickets_with(t: &Booted, key: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .where_(wl_ticketing::Ticket::SOURCE_KEY.eq(key))
        .get(t.db())
        .await
        .unwrap()
}

/// A CDD file of `client` in `status`, inserted as a fixture.
async fn file_in(t: &Booted, client: PartyId, status: CddStatus) -> CddFile {
    let mut file = CddFile {
        client_party_id: client.0,
        status,
        ..CddFile::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    file.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    file
}

/// Messages addressed to any party of `client`.
fn told_the_client(t: &Booted, client: &Client) -> usize {
    client
        .parties()
        .iter()
        .map(|party| t.notices.to_party(*party).len())
        .sum()
}

/// DW1 and DW2: an `approved` file and an `active` engagement (and a
/// `suspended` one): the MLRO's `exit` with a reason makes the file `exited`
/// and gives the partner one task "End engagement — relationship ended", due
/// in 5 working days, listing both engagements and nothing of the reason; a
/// new engagement for the client is refused with *On hold*; a manager sees
/// "Exited" and no reason.
#[renox::test]
async fn an_exit_gives_the_partner_one_task_and_refuses_new_work() {
    let t = boot_real().await;
    let (mlro, manager, partner) = (t.mlro().await, t.manager().await, t.partner().await);
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let active = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let suspended = t
        .engagement_in(&client, EngagementStatus::Suspended, Some("non_payment"))
        .await;
    let onboarding = t.engagement(&client).await;

    t.set_clock(3, 10, 0);
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", REASON)],
    )
    .await
    .assert_ok();

    let exited = t.cdd(&file).await;
    assert_eq!(exited.status, CddStatus::Exited);
    assert_eq!(exited.decision_reason.as_deref(), Some(REASON));

    // One task for the partner, due in five working days (2026-11-10).
    let tickets = tickets_with(&t, &partner_key(&exited)).await;
    assert_eq!(tickets.len(), 1, "{tickets:?}");
    let task = &tickets[0];
    assert!(task.is_open());
    assert_eq!(task.title, "End engagement — relationship ended");
    assert_eq!(task.assignee_user_id, Some(partner.id));
    assert_eq!(DUE_WORKING_DAYS, 5);
    let late = (task.due_at.expect("a due time") - dubai(10, 10, 0))
        .num_seconds()
        .abs();
    assert!(late < 5, "due 2026-11-10 10:00, off by {late}s");
    for listed in [&active, &suspended] {
        assert!(
            task.description.contains(&listed.number()),
            "{}",
            task.description
        );
    }
    assert!(
        !task.description.contains(&onboarding.number()),
        "an onboarding engagement is not ended"
    );
    for hidden in ["investigation", "Adverse", "AML", "exit"] {
        assert!(
            !task.description.contains(hidden) && !task.title.contains(hidden),
            "the task shows `{hidden}`"
        );
    }

    // The engagements run on; nothing is ended or sent by itself.
    assert_eq!(t.status(&active).await, EngagementStatus::Active);
    assert_eq!(t.status(&suspended).await, EngagementStatus::Suspended);
    assert_eq!(told_the_client(&t, &client), 0);

    // No new work: an engagement or a new CDD file is refused with On hold.
    let refused = t
        .post(
            &manager,
            "/accounting/engagements",
            &[
                ("client", &client.company.0.to_string()),
                ("title", "Audit support"),
                ("retainer_fee", "1000"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("On hold"), "{}", refused.text());
    let reopened = accounting::cdd_files::open(t.db(), None, client.company).await;
    assert!(reopened.is_err(), "an exit is final");

    // A manager sees Exited and no reason; the MLRO sees both.
    let page = t
        .get(&manager, &format!("/accounting/cdd/{}", file.id))
        .await;
    page.assert_ok().assert_see("Exited");
    assert!(!page.text().contains("investigation"), "{}", page.text());
    t.get(&mlro, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_ok()
        .assert_see("investigation");
}

/// The last `end` completes the partner's task.
#[renox::test]
async fn ending_the_last_engagement_completes_the_task() {
    let t = boot_real().await;
    let (mlro, partner) = (t.mlro().await, t.partner().await);
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let one = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let two = t
        .engagement_in(&client, EngagementStatus::Suspended, Some("non_payment"))
        .await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", REASON)],
    )
    .await
    .assert_ok();
    let key = partner_key(&t.cdd(&file).await);

    t.act(&partner, &one, "end", &[("reason", "client_request")])
        .await
        .assert_ok();
    assert!(tickets_with(&t, &key).await[0].is_open(), "one is left");
    t.act(&partner, &two, "end", &[("reason", "client_request")])
        .await
        .assert_ok();
    assert!(!tickets_with(&t, &key).await[0].is_open());
    assert_eq!(
        t.reload(&two).await.reason_code.as_deref(),
        Some("relationship_ended")
    );
}

/// DW3: a `pending_approval` file of an existing client rejected at review
/// is `rejected` and the partner has the same task.
#[renox::test]
async fn a_file_rejected_at_the_review_of_an_existing_client_gives_the_same_task() {
    let t = boot_real().await;
    let mlro = t.mlro().await;
    let partner = t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let file = file_in(&t, client.company, CddStatus::PendingApproval).await;

    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/reject", file.id),
        &[("reason", REASON)],
    )
    .await
    .assert_ok();

    assert_eq!(t.cdd(&file).await.status, CddStatus::Rejected);
    let tickets = tickets_with(&t, &partner_key(&file)).await;
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].title, "End engagement — relationship ended");
    assert_eq!(tickets[0].assignee_user_id, Some(partner.id));
    assert!(tickets[0].description.contains(&engagement.number()));
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(told_the_client(&t, &client), 0);
}

/// DW4: a `pending_approval` file at onboarding rejected declines the
/// engagement, with no partner task.
#[renox::test]
async fn a_file_rejected_at_onboarding_declines_the_engagement_without_a_task() {
    let t = boot_real().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    let file = file_in(&t, client.company, CddStatus::PendingApproval).await;

    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/reject", file.id),
        &[("reason", REASON)],
    )
    .await
    .assert_ok();

    assert_eq!(t.cdd(&file).await.status, CddStatus::Rejected);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Declined);
    assert!(tickets_with(&t, &partner_key(&file)).await.is_empty());
}

/// DW5 and DW6: with an open VAT return the partner's `end` is refused until
/// the manager cancels the return with *engagement ended*, which sends the
/// client nothing; on `end` `ACC.RUL.09` sends nothing and prepares the
/// MLRO's task with the deadline list; the client gets the neutral template
/// only when the MLRO sends it.
#[renox::test]
async fn the_end_waits_for_the_returns_and_the_client_is_told_only_by_the_mlro() {
    let t = boot_real().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t.active_engagement(&client).await;
    t.vat_task(&engagement, d(11, 28)).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", REASON)],
    )
    .await
    .assert_ok();
    let key = partner_key(&t.cdd(&file).await);
    let listed = tickets_with(&t, &key).await.remove(0);
    assert!(
        listed.description.contains(&ret.number()),
        "the open return"
    );

    // The end is refused while the return is open.
    t.set_clock(5, 10, 0);
    let refused = t
        .act(
            &crew.partner,
            &engagement,
            "end",
            &[("reason", "client_request")],
        )
        .await;
    refused.assert_status(422);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);

    // The manager cancels it: nothing is sent to the client.
    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();
    assert_eq!(told_the_client(&t, &client), 0);

    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let ended = t.reload(&engagement).await;
    assert_eq!(ended.status, EngagementStatus::Ended);
    assert_eq!(ended.reason_code.as_deref(), Some("relationship_ended"));
    assert_eq!(told_the_client(&t, &client), 0, "{:?}", t.notices.all());
    assert!(!tickets_with(&t, &key).await[0].is_open());

    // The MLRO has the task with the deadline list; the client is told when
    // the MLRO sends the neutral template.
    let notice = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(notice.prepared_for.as_deref(), Some("compliance"));
    assert!(notice.sent_at.is_none());
    t.post(
        &mlro,
        &format!("/accounting/engagements/{}/client-notice", engagement.id),
        &[("template", "ended")],
    )
    .await
    .assert_ok();
    assert_eq!(t.notices.of_type(ENDED_NOTICE).len(), 1);
    assert_eq!(t.notices.to_party(client.company).len(), 1);
}

/// The reason of an exit or a reject is for the MLRO and the deputy: the
/// deputy sees it on the file, a manager sees neither the exit reason nor the
/// reject reason on the page, in the history of the file, of the engagement or
/// of the partner's task.
#[renox::test]
async fn the_deputy_sees_the_reason_and_the_manager_sees_neither_exit_nor_reject_reason() {
    use renox::state_machine::history;

    let t = boot_real().await;
    let (mlro, deputy, manager) = (t.mlro().await, t.deputy().await, t.manager().await);
    let exited_client = t.client_named(" Exit").await;
    let rejected_client = t.client_named(" Reject").await;
    let exit_reason = "Exit: adverse media on the owner";
    let reject_reason = "Reject: forged licence";

    let approved = t.cdd_approved(&exited_client, &mlro).await;
    let exit_engagement = t
        .engagement_in(&exited_client, EngagementStatus::Active, None)
        .await;
    let pending = file_in(&t, rejected_client.company, CddStatus::PendingApproval).await;
    let reject_engagement = t
        .engagement_in(&rejected_client, EngagementStatus::Active, None)
        .await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", approved.id),
        &[("reason", exit_reason)],
    )
    .await
    .assert_ok();
    t.post(
        &deputy,
        &format!("/accounting/cdd/{}/reject", pending.id),
        &[("reason", reject_reason)],
    )
    .await
    .assert_ok();

    for (file, reason, word) in [
        (&approved, exit_reason, "adverse media"),
        (&pending, reject_reason, "forged licence"),
    ] {
        let page = format!("/accounting/cdd/{}", file.id);
        for viewer in [&mlro, &deputy] {
            t.get(viewer, &page).await.assert_ok().assert_see(word);
        }
        let shown = t.get(&manager, &page).await;
        shown.assert_ok();
        assert!(!shown.text().contains(word), "{}", shown.text());
        // History rows, the engagement's and the partner's task.
        let rows = history(t.db(), CddFile::TABLE, file.id).await.unwrap();
        assert!(
            !format!("{rows:?}").contains(word),
            "the history of {} carries the reason",
            file.id
        );
        let key = partner_key(file);
        for task in tickets_with(&t, &key).await {
            assert!(!task.description.contains(word) && !task.title.contains(word));
            assert!(!task.description.contains(reason));
        }
    }
    for engagement in [&exit_engagement, &reject_engagement] {
        let page = t
            .get(
                &manager,
                &format!("/accounting/engagements/{}", engagement.id),
            )
            .await
            .text();
        for word in ["adverse media", "forged licence"] {
            assert!(!page.contains(word), "the engagement page shows `{word}`");
        }
        let rows = history(t.db(), accounting::Engagement::TABLE, engagement.id)
            .await
            .unwrap();
        let shown = format!("{rows:?}");
        for word in ["adverse media", "forged licence"] {
            assert!(!shown.contains(word));
        }
    }
    let list = t.get(&manager, "/accounting/engagements").await.text();
    assert!(!list.contains("adverse media") && !list.contains("forged licence"));
}
