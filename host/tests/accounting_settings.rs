//! BL-ACC-24 (#348), the task, portal and notice-group settings of the 04
//! Accounting client in the real host: the waiting state reads *Waiting on
//! client*, the portal of an engagement ended on 2026-11-01 is read-only until
//! 2027-01-30 and closed from 2027-01-31 (`POR.ACL.01`), and the invoice and
//! statement notices tell the billing role when they give up (§9 *Notices*).

mod support;

use accounting::{Engagement, EngagementStatus};
use host::roles::{ACCOUNTANT, BILLING};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{accounting_booted, company, person};
use wl_kernel::Subject;
use wl_kernel::deadlines::{NewDeadlineTask, deadline_tasks};
use wl_kernel::notify::{GiveUpReaction, notification_type};

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The waiting state of a deadline task reads *Waiting on client*.
#[renox::test]
async fn the_waiting_state_reads_waiting_on_client() {
    let app = accounting_booted().await;
    let aisha = person(&app, "Aisha", ACCOUNTANT).await;
    let id = deadline_tasks(app.state())
        .create(
            app.state(),
            NewDeadlineTask::new(
                Subject::new("accounting.engagement", 12),
                "vat_return",
                "VAT Q4",
                date("2027-01-28"),
            )
            .owner_user(aisha.id),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    app.acting_as(&aisha);
    app.htmx()
        .post(&format!("/deadline-tasks/{}/actions/start", id.0), &[])
        .await
        .assert_ok();
    app.htmx()
        .post(
            &format!("/deadline-tasks/{}/actions/wait", id.0),
            &[("reason", "Waiting for the bank statements")],
        )
        .await
        .assert_ok();
    app.get("/deadline-tasks")
        .await
        .assert_ok()
        .assert_see("Waiting on client");
}

/// Done when 5 (portal): an engagement ended 2026-11-01 gives read-only access
/// until 2027-01-30 and none from 2027-01-31.
#[renox::test]
async fn the_portal_is_read_only_for_90_days_after_the_end_then_closed() {
    let app = accounting_booted().await;
    let client = company(&app, "Gulf Trading A LLC").await;
    let mut engagement = Engagement {
        client_party_id: client.0,
        title: "Bookkeeping".to_owned(),
        letter_signed: true,
        ended_on: Some(date("2026-11-01")),
        status: EngagementStatus::Ended,
        ..Engagement::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    engagement
        .insert_in_state(&mut tx, EngagementStatus::Ended, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let state = app.state();
    assert!(
        accounting::portal::is_read_only(state, client)
            .await
            .unwrap()
    );
    for (day, open) in [
        ("2026-11-02", true),
        ("2027-01-30", true),
        ("2027-01-31", false),
    ] {
        assert_eq!(
            accounting::portal::is_open(state, client, date(day))
                .await
                .unwrap(),
            open,
            "{day}"
        );
    }
}

/// The notice types and their give-up reactions of §9: statutory ones mark the
/// contact and deliver by other means; return approvals try another approver
/// then a task; engagement letters, KPI packs a task for the engagement
/// manager; invoices and statements the billing role.
#[renox::test]
async fn the_notification_types_have_the_reactions_of_the_04_groups() {
    let app = accounting_booted().await;
    let reactions = |key: &str| {
        let def = notification_type(key).unwrap_or_else(|| panic!("{key} is registered"));
        (
            def.give_up.clone(),
            def.staff_role.clone(),
            def.statutory,
            def.group.clone(),
        )
    };
    for key in ["accounting.deadline_notice", "accounting.request_list"] {
        let (_, _, statutory_flag, group) = reactions(key);
        assert!(statutory_flag, "{key}");
        assert_eq!(group, "statutory");
    }
    let approval = reactions("accounting.return_for_approval");
    assert_eq!(
        approval.0,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::AlternativeRecipient,
            GiveUpReaction::StaffTask
        ]
    );
    assert_eq!(approval.1.as_deref(), Some("engagement_manager"));
    for key in ["accounting.kpi_published", "accounting.kpi_withdrawn"] {
        let (give_up, role, _, _) = reactions(key);
        assert_eq!(
            give_up,
            [
                GiveUpReaction::MarkContactInvalid,
                GiveUpReaction::StaffTask
            ],
            "{key}"
        );
        assert_eq!(role.as_deref(), Some("engagement_manager"), "{key}");
    }
    assert_eq!(
        reactions("accounting.letter_reminder").1.as_deref(),
        Some("engagement_manager")
    );

    // Invoice and statement notices tell billing: the seed replaced the
    // groups' receivables role.
    let rows = wl_notify::NotificationType::query()
        .get(app.db())
        .await
        .unwrap();
    let money: Vec<_> = rows
        .iter()
        .filter(|r| ["invoices", "statements"].contains(&r.group_key.as_str()))
        .collect();
    assert!(!money.is_empty());
    for row in money {
        assert_eq!(row.staff_role.as_deref(), Some(BILLING), "{}", row.key);
    }
}

/// The demo client lists the UAE public holidays: the 20 business days of a
/// voluntary disclosure found on Thursday 2026-11-05 skip the National Day
/// holidays of 2026-12-02 and 2026-12-03 and end on 2026-12-07, where
/// without them they would end on 2026-12-03.
#[renox::test]
async fn the_voluntary_disclosure_deadline_skips_the_public_holidays_of_the_client() {
    let app = accounting_booted().await;
    let due = accounting::corrections::disclosure_due(app.state(), date("2026-11-05"));
    assert_eq!(due, date("2026-12-07"));
    let listed: Vec<NaiveDate> = wl_kernel::calendar::calendar(app.state())
        .holidays()
        .collect();
    assert!(listed.contains(&date("2026-12-02")));
    assert!(listed.contains(&date("2026-12-03")));
}
