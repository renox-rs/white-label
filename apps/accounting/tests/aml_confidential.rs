//! Tipping off (`ACC.ACL.05`, `ACC.ACL.14`): the MLRO's tasks about a client
//! (the CDD review after a report, the engagement decision, the deadline
//! notice of a restricted end) are tickets of the confidential category. The
//! MLRO and the deputy find them; a manager and a partner who hold
//! `tickets.view` get a 404 on the ticket and do not find it in the list,
//! the search or the exports. With the real wl-ticketing and wl-parties.

mod support;

use accounting::aml_cases::{decision_key, review_key};
use accounting::deadline_notices::{DeadlineNotice, prepared_key};
use accounting::{AmlCase, AmlStatus, EngagementStatus};
use renox::prelude::*;
use renox::state_machine::history;
use wl_ticketing::Ticket;

use support::{Booted, boot_real};

/// The tickets with the source key `key`.
async fn tickets_with(t: &Booted, key: &str) -> Vec<Ticket> {
    Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key))
        .get(t.db())
        .await
        .unwrap()
}

/// A partner who also holds `tickets.view`, to show the ticket list does not
/// depend on the host leaving that permission out of the role.
async fn partner_with_tickets(t: &Booted) -> User {
    t.user_with_grants(
        "Paula",
        &[
            "accounting.engagements.view",
            "accounting.engagements.end",
            "tickets.view",
            "tickets.tasks.view",
        ],
    )
    .await
}

/// Every way the user can look at ticket `id`: the page, the list, the
/// search of the list and the CSV export. True when the user sees it.
async fn sees(t: &Booted, user: &User, ticket: &Ticket) -> bool {
    let page = t.get(user, &format!("/tickets/{}", ticket.id)).await;
    let list = t.get(user, "/tickets").await;
    let search = t
        .get(user, &format!("/tickets?search={}", ticket.number))
        .await;
    let export = t.get(user, "/tickets?export=csv").await;
    // The row's link: the search box echoes the number it was given.
    let link = format!("/tickets/{}\"", ticket.id);
    let found =
        |r: &renox::testing::TestResponse| r.status.as_u16() == 200 && r.text().contains(&link);
    let (found_in_list, found_in_search) = (found(&list), found(&search));
    let shown = page.status.as_u16() == 200;
    assert!(
        !found(&export) || shown,
        "the export lists {}",
        ticket.number
    );
    assert_eq!(
        shown, found_in_list,
        "page and list disagree for {}",
        ticket.number
    );
    if !shown {
        assert_eq!(
            page.status.as_u16(),
            404,
            "a ticket that is not seen answers 404"
        );
        assert!(!found_in_search);
        assert!(!page.text().contains(&ticket.title));
    }
    shown
}

/// A case under review whose report gave the MLRO the two tasks.
async fn reported(t: &Booted, manager: &User, mlro: &User) -> (AmlCase, Ticket, Ticket) {
    let client = t.client().await;
    t.cdd_approved(&client, mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
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
    t.set_clock(5, 10, 0);
    t.post(
        mlro,
        &format!("/accounting/aml-cases/{}/report", case.id),
        &[("reference", "GOAML-2026-0042"), ("date", "2026-11-05")],
    )
    .await
    .assert_ok();
    let case = t.aml_case(&case).await;
    assert_eq!(case.status, AmlStatus::Reported);
    let review = tickets_with(t, &review_key(&case)).await.remove(0);
    let decision = tickets_with(t, &decision_key(&case)).await.remove(0);
    (case, review, decision)
}

/// The review task and the decision task are confidential: the MLRO and the
/// deputy see them; a manager and a partner (both with `tickets.view`) get a
/// 404 and do not find them; nothing the task says shows in a title.
#[renox::test]
async fn the_mlros_tasks_are_invisible_to_a_manager_and_a_partner() {
    let t = boot_real().await;
    let (mlro, deputy, manager) = (t.mlro().await, t.deputy().await, t.manager().await);
    let (host_partner, partner) = (t.partner().await, partner_with_tickets(&t).await);
    let (case, review, decision) = reported(&t, &manager, &mlro).await;

    for ticket in [&review, &decision] {
        assert!(ticket.confidential, "{} is confidential", ticket.title);
        for title_word in ["report", "AML", "sanction", "concern", "goAML"] {
            assert!(
                !ticket
                    .title
                    .to_lowercase()
                    .contains(&title_word.to_lowercase()),
                "the title {:?} carries {title_word}",
                ticket.title
            );
        }
        assert!(sees(&t, &mlro, ticket).await, "the MLRO");
        assert!(sees(&t, &deputy, ticket).await, "the deputy");
        assert!(!sees(&t, &manager, ticket).await, "the manager");
        assert!(!sees(&t, &partner, ticket).await, "the partner");
        // The partner role of the host holds no tickets.view at all.
        t.get(&host_partner, &format!("/tickets/{}", ticket.id))
            .await
            .assert_status(403);
    }

    // The manager cannot act on it either: no comment, start or assignment.
    for action in ["start", "resolve"] {
        let response = t
            .post(
                &manager,
                &format!("/tickets/{}/actions/{action}", review.id),
                &[("note", "x")],
            )
            .await;
        assert_ne!(response.status.as_u16(), 200, "{action}");
        assert_ne!(response.status.as_u16(), 303, "{action}");
    }
    assert!(
        tickets_with(&t, &review_key(&case)).await[0].is_open(),
        "still open"
    );

    // The MLRO's and the deputy's views are audited, the manager's refused one is not a view.
    let entries = renox::audit::for_subject(t.db(), Ticket::TABLE, review.id, 20)
        .await
        .unwrap();
    let viewers: Vec<Option<i64>> = entries
        .iter()
        .filter(|e| e.action == "ticket.confidential_viewed")
        .map(|e| e.user_id)
        .collect();
    assert!(viewers.contains(&Some(mlro.id)) && viewers.contains(&Some(deputy.id)));
    assert!(!viewers.contains(&Some(manager.id)));
}

/// The tickets' history rows and audit rows carry the title, never the
/// goAML reference or the word AML, so nothing there tips anybody off either.
#[renox::test]
async fn the_history_of_the_tasks_names_no_case() {
    let t = boot_real().await;
    let (mlro, manager) = (t.mlro().await, t.manager().await);
    let (_case, review, decision) = reported(&t, &manager, &mlro).await;
    for ticket in [&review, &decision] {
        let rows = history(t.db(), Ticket::TABLE, ticket.id).await.unwrap();
        for row in rows {
            let shown = format!("{row:?}");
            assert!(!shown.contains("GOAML"), "{shown}");
            assert!(!shown.contains("Payments that do not fit"), "{shown}");
        }
    }
}

/// The MLRO's task for the deadline notice of a restricted end is
/// confidential too: a manager does not find it.
#[renox::test]
async fn the_deadline_notice_task_of_a_restricted_end_is_confidential() {
    let t = boot_real().await;
    let (mlro, manager, partner) = (t.mlro().await, t.manager().await, t.partner().await);
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", "AML concern")],
    )
    .await
    .assert_ok();
    t.set_clock(15, 10, 0);
    t.act(
        &partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let notice = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(notice.prepared_for.as_deref(), Some("compliance"));
    let task = tickets_with(&t, &prepared_key(notice.id)).await.remove(0);
    assert!(task.confidential);
    assert!(sees(&t, &mlro, &task).await);
    assert!(!sees(&t, &manager, &task).await);
}
