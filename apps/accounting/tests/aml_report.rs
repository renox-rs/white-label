//! Acceptance checks of #335 (BL-ACC-11): the MLRO records a goAML report on
//! a case under review; the client's approved CDD file goes to `review_due`
//! (any other state gets the task without a transition), the compliance
//! officer gets a task, the partner is told only that the MLRO needs to talk,
//! and the MLRO decides, from a task due in a working day, whether the
//! engagements continue, are suspended or end. With the real wl-ticketing
//! and wl-parties.

mod support;

use accounting::aml_cases::{AML_CONCERN, PARTNER_NOTICE, decision_key, review_key};
use accounting::deadline_notices::DEADLINE_NOTICE;
use accounting::{AmlCase, AmlDecision, AmlStatus, CddFile, CddStatus, EngagementStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::state_machine::history;
use renox::testing::TestResponse;
use wl_kernel::PartyStatus;
use wl_kernel::notify::NotifyRecipient;
use wl_parties::BlockRecord;

use support::{Booted, Client, boot_real, dubai};

/// The reference of the report.
const REFERENCE: &str = "GOAML-2026-0042";

/// The tickets with the source key `key`.
async fn tickets_with(t: &Booted, key: &str) -> Vec<wl_ticketing::Ticket> {
    wl_ticketing::Ticket::query()
        .where_(wl_ticketing::Ticket::SOURCE_KEY.eq(key))
        .get(t.db())
        .await
        .unwrap()
}

/// Messages addressed to any party of `client`.
fn told_the_client(t: &Booted, client: &Client) -> usize {
    client
        .parties()
        .iter()
        .map(|party| t.notices.to_party(*party).len())
        .sum()
}

/// A concern about `client` raised by `manager` and taken by `mlro`: the case
/// is `under_review`.
async fn under_review(
    t: &Booted,
    manager: &User,
    mlro: &User,
    engagement: &accounting::Engagement,
) -> AmlCase {
    t.post(
        manager,
        &format!("/accounting/engagements/{}/aml-concern", engagement.id),
        &[("facts", "Payments that do not fit the trade")],
    )
    .await
    .assert_ok();
    let case = t.aml_cases().await.remove(0);
    t.post(
        mlro,
        &format!("/accounting/aml-cases/{}/review", case.id),
        &[],
    )
    .await
    .assert_ok();
    t.aml_case(&case).await
}

/// A report on the case at 10:00 on 2026-11-05.
async fn report(t: &Booted, mlro: &User, case: &AmlCase) -> TestResponse {
    t.set_clock(5, 10, 0);
    t.post(
        mlro,
        &format!("/accounting/aml-cases/{}/report", case.id),
        &[("reference", REFERENCE), ("date", "2026-11-05")],
    )
    .await
}

/// DW1 and DW2: `report` with "GOAML-2026-0042" and a date makes the case
/// `reported`, the approved file `review_due`, gives the compliance officer a
/// task and the partner a notice without case detail; without a reference it
/// is refused.
#[renox::test]
async fn a_report_flags_the_file_and_tells_the_partner_nothing_of_the_case() {
    let t = boot_real().await;
    let (mlro, manager, partner) = (t.mlro().await, t.manager().await, t.partner().await);
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let case = under_review(&t, &manager, &mlro, &engagement).await;
    assert_eq!(case.status, AmlStatus::UnderReview);
    let path = format!("/accounting/aml-cases/{}/report", case.id);

    // A reference and a date are required.
    t.post(&mlro, &path, &[("reference", " "), ("date", "2026-11-05")])
        .await
        .assert_status(422);
    t.post(&mlro, &path, &[("reference", REFERENCE), ("date", "")])
        .await
        .assert_status(422);
    assert_eq!(t.aml_case(&case).await.status, AmlStatus::UnderReview);
    assert_eq!(t.cdd(&file).await.status, CddStatus::Approved);

    report(&t, &mlro, &case).await.assert_ok();
    let reported = t.aml_case(&case).await;
    assert_eq!(reported.status, AmlStatus::Reported);
    assert_eq!(reported.goaml_reference.as_deref(), Some(REFERENCE));
    assert_eq!(
        reported.goaml_date.map(|d| d.to_string()).as_deref(),
        Some("2026-11-05")
    );
    assert_eq!(reported.reported_by, Some(mlro.id));

    // The approved file is flagged for review.
    assert_eq!(t.cdd(&file).await.status, CddStatus::ReviewDue);
    let rows = history(t.db(), CddFile::TABLE, file.id).await.unwrap();
    assert!(rows.iter().any(|row| row.transition == "flag_review"));

    // The compliance officer's task.
    let tasks = tickets_with(&t, &review_key(&reported)).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Review CDD after report");
    assert_eq!(tasks[0].assignee_user_id, Some(mlro.id));
    assert!(tasks[0].is_open());

    // The partner is told that the MLRO needs to talk, and nothing more.
    let notices = t.notices.of_type(PARTNER_NOTICE);
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        [NotifyRecipient::User { id: partner.id }]
    );
    let shown = format!("{:?}", notices[0]);
    for hidden in [REFERENCE, "AML", "report", "sanction", "concern"] {
        assert!(
            !shown.contains(hidden),
            "the notice shows `{hidden}`: {shown}"
        );
    }
    assert!(shown.contains("Gulf Trading"), "{shown}");
    t.get(&partner, &format!("/accounting/aml-cases/{}", case.id))
        .await
        .assert_not_found();
    let page = t
        .get(
            &partner,
            &format!("/accounting/engagements/{}", engagement.id),
        )
        .await;
    page.assert_ok();
    assert!(!page.text().contains(REFERENCE), "{}", page.text());
    assert_eq!(told_the_client(&t, &client), 0);

    // The MLRO's task to record the engagement decision, due in a working
    // day (Thursday 2026-11-05 to Friday 2026-11-06).
    let decision = tickets_with(&t, &decision_key(&reported)).await;
    assert_eq!(decision.len(), 1);
    assert_eq!(decision[0].assignee_user_id, Some(mlro.id));
    let late = (decision[0].due_at.expect("a due time") - dubai(6, 10, 0))
        .num_seconds()
        .abs();
    assert!(late < 5, "due 2026-11-06 10:00, off by {late}s");
}

/// DW3: the MLRO records *suspend*: the engagement is `suspended` with *on
/// hold*, the client company is blocked with `aml_concern`, the client gets
/// no message; the decision needs the MLRO and a reported case, once.
#[renox::test]
async fn the_suspend_decision_suspends_with_on_hold_and_blocks_for_aml_concern() {
    let t = boot_real().await;
    let (mlro, manager, partner) = (t.mlro().await, t.manager().await, t.partner().await);
    let client = t.client().await;
    t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let case = under_review(&t, &manager, &mlro, &engagement).await;
    let path = format!("/accounting/aml-cases/{}/decision", case.id);

    // Not before the report, not by anybody else, and a known decision.
    t.post(&mlro, &path, &[("decision", "suspend")])
        .await
        .assert_status(422);
    report(&t, &mlro, &case).await.assert_ok();
    t.post(&partner, &path, &[("decision", "suspend")])
        .await
        .assert_not_found();
    t.post(&mlro, &path, &[("decision", "sanctions")])
        .await
        .assert_status(422);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);

    t.post(
        &mlro,
        &path,
        &[("decision", "suspend"), ("note", "Pending the FIU")],
    )
    .await
    .assert_ok();
    let decided = t.aml_case(&case).await;
    assert_eq!(decided.engagement_decision, Some(AmlDecision::Suspend));
    assert_eq!(decided.decision_note.as_deref(), Some("Pending the FIU"));
    assert_eq!(decided.decided_by, Some(mlro.id));

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    assert!(!suspended.may_message_client());
    assert_eq!(
        t.party_row(client.company).await.status,
        PartyStatus::Blocked
    );
    let records = BlockRecord::open_for(t.db(), client.company.0)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason_code, AML_CONCERN);
    assert!(records[0].restricted);
    assert_eq!(told_the_client(&t, &client), 0, "{:?}", t.notices.all());
    assert!(t.notices.of_type(DEADLINE_NOTICE).is_empty());
    assert!(!tickets_with(&t, &decision_key(&decided)).await[0].is_open());

    // The decision is recorded once.
    t.post(&mlro, &path, &[("decision", "continue")])
        .await
        .assert_status(422);
    assert_eq!(
        t.aml_case(&case).await.engagement_decision,
        Some(AmlDecision::Suspend)
    );
}

/// *Continue* changes nothing but the record.
#[renox::test]
async fn the_continue_decision_changes_nothing() {
    let t = boot_real().await;
    let (mlro, manager) = (t.mlro().await, t.manager().await);
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let case = under_review(&t, &manager, &mlro, &engagement).await;
    report(&t, &mlro, &case).await.assert_ok();

    t.post(
        &mlro,
        &format!("/accounting/aml-cases/{}/decision", case.id),
        &[("decision", "continue")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.aml_case(&case).await.engagement_decision,
        Some(AmlDecision::Continue)
    );
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(t.cdd(&file).await.status, CddStatus::ReviewDue);
    assert_eq!(
        t.party_row(client.company).await.status,
        PartyStatus::Active
    );
}

/// The *exit* decision exits the CDD file (the reason is the MLRO's alone)
/// and the partner gets the task to end the engagement.
#[renox::test]
async fn the_exit_decision_exits_the_client_through_cdd() {
    let t = boot_real().await;
    let (mlro, manager, partner) = (t.mlro().await, t.manager().await, t.partner().await);
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let case = under_review(&t, &manager, &mlro, &engagement).await;
    report(&t, &mlro, &case).await.assert_ok();

    t.post(
        &mlro,
        &format!("/accounting/aml-cases/{}/decision", case.id),
        &[("decision", "exit")],
    )
    .await
    .assert_ok();
    let exited = t.cdd(&file).await;
    assert_eq!(exited.status, CddStatus::Exited);
    assert!(
        exited
            .decision_reason
            .as_deref()
            .is_some_and(|reason| reason.contains(&case.number())),
        "{:?}",
        exited.decision_reason
    );
    let tasks = tickets_with(&t, &accounting::exits::partner_key(&exited)).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "End engagement — relationship ended");
    assert_eq!(tasks[0].assignee_user_id, Some(partner.id));
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(told_the_client(&t, &client), 0);
}

/// DW4: with the file already `review_due`, `report` flags nothing and
/// creates the "review CDD after report" task instead; so for a file in
/// progress.
#[renox::test]
async fn a_file_not_approved_gets_the_task_without_a_transition() {
    let t = boot_real().await;
    let (mlro, manager) = (t.mlro().await, t.manager().await);
    for status in [CddStatus::ReviewDue, CddStatus::InProgress] {
        let client = t.client_named(status.as_str()).await;
        let mut file = CddFile {
            client_party_id: client.company.0,
            status,
            ..CddFile::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        file.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let engagement = t
            .engagement_in(&client, EngagementStatus::Active, None)
            .await;
        t.post(
            &manager,
            &format!("/accounting/engagements/{}/aml-concern", engagement.id),
            &[("facts", "Payments that do not fit the trade")],
        )
        .await
        .assert_ok();
        let case = t
            .aml_cases()
            .await
            .into_iter()
            .find(|c| c.client() == client.company)
            .expect("the case");
        t.post(
            &mlro,
            &format!("/accounting/aml-cases/{}/review", case.id),
            &[],
        )
        .await
        .assert_ok();
        report(&t, &mlro, &case).await.assert_ok();

        assert_eq!(t.aml_case(&case).await.status, AmlStatus::Reported);
        assert_eq!(t.cdd(&file).await.status, status, "no transition");
        let rows = history(t.db(), CddFile::TABLE, file.id).await.unwrap();
        assert!(rows.iter().all(|row| row.transition != "flag_review"));
        let tasks = tickets_with(&t, &review_key(&case)).await;
        assert_eq!(tasks.len(), 1, "{status:?}");
        assert_eq!(tasks[0].title, "Review CDD after report");
        assert_eq!(tasks[0].assignee_user_id, Some(mlro.id));
    }
}
