//! BL-CON-19 (#298): the client portal turns read-only when a project's final
//! account is closed (`POR.ACL.01`). Given project P closed on 2028-01-31,
//! its client contact can read P's documents until 2028-04-30 and cannot
//! upload; on 2028-05-01 P is no longer visible. The client's other projects
//! are not touched.

mod support;

use construction::client_portal::{
    self, CLOSE_AFTER_DAYS, ClientAccess, access_on, closed_on, has_open_project, upload_problem,
    visible_projects,
};
use construction::{Ipa, IpaKind, IpaStatus, Project, ProjectStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::invoicing::InvoiceStatus;

use support::{Booted, CLIENT_PARTY, boot, date};

/// A terminated project whose final account IPA is invoiced: ready to
/// settle.
async fn ready_to_settle(t: &Booted, manager: &User) -> Project {
    let project = t.project(ProjectStatus::Terminated, Some(manager)).await;
    let invoice = t.invoices.put(InvoiceStatus::Issued);
    let mut account = Ipa {
        kind: IpaKind::FinalAccount,
        status: IpaStatus::Invoiced,
        valuation_on: Some(date(2027, 12, 31)),
        claimed_amount: 9_000_000,
        certified_amount: Some(8_500_000),
        invoice_id: Some(invoice),
        ..Ipa::new(project.id)
    };
    let mut tx = t.db().begin().await.unwrap();
    account
        .insert_in_state(&mut tx, IpaStatus::Invoiced, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    project
}

/// Done when: P settled on 2028-01-31 is readable through 2028-04-30 with
/// no upload, and hidden on 2028-05-01; Q, another project of the same
/// client still running, stays fully visible.
#[renox::test]
async fn bl_con_19_a_closed_project_is_read_only_for_90_days_then_hidden() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let p = ready_to_settle(&t, &a).await;
    let q = t.project(ProjectStatus::InProgress, Some(&a)).await;
    assert_eq!(p.client_party_id, q.client_party_id);

    // Before the close, P is the client's to read and upload to.
    assert_eq!(access_on(&p, date(2028, 1, 30)), ClientAccess::Full);

    t.on(date(2028, 1, 31), async {}).await;
    t.post(&a, &format!("/construction/projects/{}/settle", p.id), &[])
        .await
        .assert_ok();
    let p = t.reload(&p).await;
    assert_eq!(p.status, ProjectStatus::Closed);
    assert_eq!(closed_on(&p), Some(date(2028, 1, 31)));

    // The host is told, with the day it closed.
    let closed = t.closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].project, p.subject());
    assert_eq!(closed[0].client, CLIENT_PARTY);
    assert_eq!(closed[0].closed_on, date(2028, 1, 31));
    assert_eq!(closed[0].transition, "settle_final_account");

    // Read-only through day 90.
    let until = date(2028, 4, 30);
    assert_eq!(CLOSE_AFTER_DAYS, 90);
    for day in [date(2028, 1, 31), date(2028, 3, 1), until] {
        let access = access_on(&p, day);
        assert_eq!(access, ClientAccess::ReadOnly { until }, "{day}");
        assert!(access.visible());
        assert!(!access.may_upload(), "no upload on {day}");
        assert!(
            upload_problem(&p, day).is_some_and(|text| text.contains("2028-04-30")),
            "{day}"
        );
    }
    let seen = visible_projects(t.db(), CLIENT_PARTY, until).await.unwrap();
    let ids: Vec<(i64, ClientAccess)> = seen.iter().map(|(p, a)| (p.id, *a)).collect();
    assert_eq!(
        ids,
        vec![
            (p.id, ClientAccess::ReadOnly { until }),
            (q.id, ClientAccess::Full)
        ]
    );

    // Day 91: P is gone, Q is untouched.
    let after = date(2028, 5, 1);
    assert_eq!(access_on(&p, after), ClientAccess::Hidden);
    assert!(upload_problem(&p, after).is_some());
    let seen = visible_projects(t.db(), CLIENT_PARTY, after).await.unwrap();
    let ids: Vec<(i64, ClientAccess)> = seen.iter().map(|(p, a)| (p.id, *a)).collect();
    assert_eq!(ids, vec![(q.id, ClientAccess::Full)]);
    assert_eq!(access_on(&q, after), ClientAccess::Full);
    assert!(upload_problem(&q, after).is_none());

    // Q still runs, so the client's portal grant must stay active.
    assert!(has_open_project(t.db(), CLIENT_PARTY).await.unwrap());
}

/// `close` after the defects liability period closes the final account too:
/// the same window, counted from the day it closed; with no other project
/// left, the client's grant may end.
#[renox::test]
async fn bl_con_19_a_project_closed_after_the_defects_period_is_read_only_too() {
    let t = boot().await;
    let a = t.project_manager("Amal").await;
    let p = t
        .project(ProjectStatus::PracticalCompletion, Some(&a))
        .await;
    Project::query()
        .where_(Project::ID.eq(p.id))
        .update(
            t.db(),
            &[
                ("completed_on", &Some(date(2027, 1, 15))),
                ("defects_liability_ends_on", &Some(date(2028, 1, 15))),
            ],
        )
        .await
        .unwrap();

    t.on(date(2028, 1, 31), async {}).await;
    t.post(&a, &format!("/construction/projects/{}/close", p.id), &[])
        .await
        .assert_ok();
    let p = t.reload(&p).await;
    assert_eq!(p.status, ProjectStatus::Closed);
    assert_eq!(
        client_portal::readable_until(date(2028, 1, 31)),
        date(2028, 4, 30)
    );
    assert_eq!(
        access_on(&p, date(2028, 4, 30)),
        ClientAccess::ReadOnly {
            until: date(2028, 4, 30)
        }
    );
    assert_eq!(access_on(&p, date(2028, 5, 1)), ClientAccess::Hidden);
    let closed = t.closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].transition, "close");
    assert!(!has_open_project(t.db(), CLIENT_PARTY).await.unwrap());
}
