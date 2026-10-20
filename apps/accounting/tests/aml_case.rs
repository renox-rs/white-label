//! Acceptance checks of #334 (BL-ACC-10, `ACC.RUL.12`) and the AML case
//! base of #335: a confirmed sanctions match opens an AML case linked to the
//! screening and to the freeze and report tasks, the client company is
//! blocked (by wl-parties, or by the rule when it did not), the engagement
//! is suspended with *on hold* and its retainer paused, nothing goes to the
//! client; the MLRO takes the case and closes it once both tasks are done,
//! and nobody else sees it (`ACC.ACL.05`). With the real wl-parties,
//! wl-ticketing and wl-invoicing.

mod support;

use std::time::Duration;

use accounting::aml_cases::{self, CASE_NOTICE, RULE};
use accounting::{AmlKind, AmlStatus, Engagement, EngagementStatus, retainer};
use renox::prelude::*;
use wl_kernel::automation::RunOutcome;
use wl_kernel::compliance::{ComplianceAlert, is_critical};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::subscriptions::{SubscriptionStatus, subscriptions};
use wl_kernel::{PartyKind, PartyStatus, ScreeningId, ScreeningMatchConfirmed};
use wl_parties::{
    BlockRecord, ComplianceTaskKind, NewRelationship, RelationshipType, Screening, relate,
};

use support::{Booted, Client, boot_real, boot_unrelated, d, dubai};

/// Falcon Trading LLC with its UBO Omar K., a director and a signatory, all
/// on its CDD file; with `related` Omar is also its UBO in wl-parties'
/// relationships (what the related-person block follows). Its engagement is
/// `active` with the retainer started.
async fn falcon(t: &Booked, related: bool) -> (Client, Engagement) {
    let client = Client {
        company: t.party("Falcon Trading LLC", PartyKind::Company).await,
        ubo: t.party("Omar K.", PartyKind::Person).await,
        director: t.party("Lina Saeed", PartyKind::Person).await,
        signatory: t.party("Karim Nasser", PartyKind::Person).await,
    };
    t.cdd_prepared(&client).await;
    if related {
        relate(
            t.state(),
            NewRelationship {
                from: client.ubo.0,
                to: client.company.0,
                relation_type: RelationshipType::UboOf,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let started = t
        .run(retainer::start(t.state(), &engagement, d(11, 1)))
        .await
        .unwrap();
    assert!(started.is_some(), "the retainer started");
    (client, engagement)
}

type Booked = Booted;

/// `party` is flagged on 2026-11-03 at 14:00 and the match confirmed, with
/// the second review, on 2026-11-04 at 09:00 (Asia/Dubai).
async fn confirmed(t: &Booted, party: wl_kernel::PartyId, a: &User, b: &User) -> Screening {
    t.set_clock(3, 14, 0);
    let screening = t.screen_possible_match(party).await;
    t.set_clock(4, 9, 0);
    t.confirm_match(&screening, a, b).await;
    screening
}

/// Every message addressed to one of the parties of `client`.
fn told_the_client(t: &Booted, client: &Client) -> usize {
    client
        .parties()
        .iter()
        .map(|party| t.notices.to_party(*party).len())
        .sum()
}

/// DW1 and DW2: UBO "Omar K." of "Falcon Trading LLC" flagged 2026-11-03
/// 14:00 and confirmed 2026-11-04 09:00: an AML case is `open` with the
/// screening, the freeze task and the report task (both due 2026-11-04
/// 14:00) linked, and the MLRO and deputy are notified; the engagement is
/// `suspended` with *on hold*, its retainer `paused`, and nothing reaches the
/// client.
#[renox::test]
async fn a_confirmed_match_opens_a_case_with_the_tasks_and_suspends_the_client() {
    let t = boot_real().await;
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let (client, engagement) = falcon(&t, true).await;
    assert!(is_critical(RULE), "ACC.RUL.12 is compliance-critical");

    let screening = confirmed(&t, client.ubo, &mlro, &deputy).await;

    let cases = t.aml_cases().await;
    assert_eq!(cases.len(), 1, "{cases:?}");
    let case = &cases[0];
    assert_eq!(case.status, AmlStatus::Open);
    assert_eq!(case.kind, AmlKind::Sanctions);
    assert_eq!(case.client(), client.company);
    assert_eq!(case.raised_by, None, "opened by the system");
    assert!(case.facts.contains("Omar K."), "{}", case.facts);
    assert!(case.facts.contains("Falcon Trading LLC"), "{}", case.facts);
    assert!(case.facts.contains("a UBO"), "{}", case.facts);
    assert!(case.facts.contains("QDi.001"), "{}", case.facts);
    assert_eq!(
        case.block_note, None,
        "wl-parties blocked the company itself"
    );

    let matches = case.matches(t.db()).await.unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].screening_id, screening.id);
    assert_eq!(matches[0].matched_party_id, client.ubo.0);
    let tasks = t.compliance_tasks(&screening).await;
    assert_eq!(tasks.len(), 2);
    for task in &tasks {
        let late = (task.due_at - dubai(4, 14, 0)).num_seconds().abs();
        assert!(late < 5, "due 2026-11-04 14:00, off by {late}s");
    }
    let id_of = |kind| tasks.iter().find(|task| task.kind == kind).map(|t| t.id);
    assert_eq!(matches[0].freeze_task_id, id_of(ComplianceTaskKind::Freeze));
    assert_eq!(matches[0].report_task_id, id_of(ComplianceTaskKind::Report));
    assert!(matches[0].freeze_task_id.is_some() && matches[0].report_task_id.is_some());

    // The MLRO and deputy are told; nobody else.
    let notices = t.notices.of_type(CASE_NOTICE);
    let told: Vec<_> = notices.iter().flat_map(|n| n.recipients.clone()).collect();
    assert!(told.contains(&NotifyRecipient::User { id: mlro.id }));
    assert!(told.contains(&NotifyRecipient::User { id: deputy.id }));
    assert_eq!(told.len(), 2, "{told:?}");

    // The block suspends the engagement with the neutral reason, pauses the
    // retainer, and no message goes to the client.
    assert_eq!(
        t.party_row(client.company).await.status,
        PartyStatus::Blocked
    );
    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    let retainers = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(retainers[0].status, SubscriptionStatus::Paused);
    assert_eq!(told_the_client(&t, &client), 0, "{:?}", t.notices.all());

    // A second confirmation event for the same screening opens nothing.
    t.state()
        .emit(ScreeningMatchConfirmed {
            party: client.ubo,
            screening: ScreeningId(screening.id),
        })
        .await
        .unwrap();
    assert_eq!(t.aml_cases().await.len(), 1);
    assert_eq!(t.notices.of_type(CASE_NOTICE).len(), notices.len());
}

/// DW3: with wl-parties' related-person block off, the rule blocks Falcon
/// Trading with `sanctions` itself and notes it on the case.
#[renox::test]
async fn the_rule_blocks_the_client_itself_when_the_related_block_is_off() {
    let t = boot_unrelated().await;
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let (client, engagement) = falcon(&t, true).await;

    let screening = confirmed(&t, client.ubo, &mlro, &deputy).await;

    assert_eq!(
        t.party_row(client.ubo).await.status,
        PartyStatus::Blocked,
        "the matched person is blocked by wl-parties"
    );
    assert_eq!(
        t.party_row(client.company).await.status,
        PartyStatus::Blocked,
        "the rule blocked the client company"
    );
    let records = BlockRecord::open_for(t.db(), client.company.0)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason_code, "sanctions");
    assert!(records[0].restricted);
    assert_eq!(records[0].screening_id, Some(screening.id));

    let cases = t.aml_cases().await;
    assert_eq!(cases.len(), 1);
    let note = cases[0].block_note.as_deref().expect("noted on the case");
    assert!(note.contains("ACC.RUL.12"), "{note}");
    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    assert_eq!(told_the_client(&t, &client), 0);
}

/// A second confirmed match on another person of the same client adds to
/// the open case: one open sanctions case per client company.
#[renox::test]
async fn another_person_of_the_same_client_adds_to_the_open_case() {
    let t = boot_real().await;
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let (client, _engagement) = falcon(&t, true).await;

    confirmed(&t, client.ubo, &mlro, &deputy).await;
    let second = confirmed(&t, client.director, &mlro, &deputy).await;

    let cases = t.aml_cases().await;
    assert_eq!(cases.len(), 1, "{cases:?}");
    let matches = cases[0].matches(t.db()).await.unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[1].screening_id, second.id);
    assert_eq!(matches[1].matched_party_id, client.director.0);
    assert!(cases[0].facts.contains("Lina Saeed"), "{}", cases[0].facts);
    assert!(cases[0].facts.contains("a director"), "{}", cases[0].facts);
}

/// DW4: the MLRO runs `review`; `close` with the report task still open is
/// refused; with both tasks done and a reason it is `closed_no_action` and
/// the client stays blocked. `close` is not offered from `open`.
#[renox::test]
async fn the_mlro_reviews_and_closes_once_both_tasks_are_done() {
    let t = boot_real().await;
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let (client, _engagement) = falcon(&t, true).await;
    let screening = confirmed(&t, client.ubo, &mlro, &deputy).await;
    let case = t.aml_cases().await.remove(0);
    let path = |action: &str| format!("/accounting/aml-cases/{}/{action}", case.id);
    let reason = [("reason", "Reviewed: the list entry is another person")];

    // From `open`, `close` is not offered.
    t.post(&mlro, &path("close"), &reason)
        .await
        .assert_status(422);
    t.post(&mlro, &path("review"), &[]).await.assert_ok();
    assert_eq!(t.aml_case(&case).await.status, AmlStatus::UnderReview);

    // A reason is required; an open task refuses.
    t.post(&mlro, &path("close"), &[("reason", " ")])
        .await
        .assert_status(422);
    let refused = t.post(&mlro, &path("close"), &reason).await;
    refused.assert_status(422);
    assert!(refused.text().contains("report task"), "{}", refused.text());
    assert_eq!(t.aml_case(&case).await.status, AmlStatus::UnderReview);

    t.finish_task(&screening, ComplianceTaskKind::Freeze).await;
    let refused = t.post(&mlro, &path("close"), &reason).await;
    refused.assert_status(422);
    assert!(refused.text().contains("report task"), "{}", refused.text());
    assert!(
        !refused.text().contains("freeze task"),
        "{}",
        refused.text()
    );

    t.finish_task(&screening, ComplianceTaskKind::Report).await;
    t.post(&mlro, &path("close"), &reason).await.assert_ok();
    let closed = t.aml_case(&case).await;
    assert_eq!(closed.status, AmlStatus::ClosedNoAction);
    assert_eq!(
        closed.closing_reason.as_deref(),
        Some("Reviewed: the list entry is another person")
    );
    assert_eq!(closed.closed_by, Some(mlro.id));
    assert_eq!(
        t.party_row(client.company).await.status,
        PartyStatus::Blocked,
        "closing the case does not unblock the client"
    );

    // A closed case is final.
    t.post(&mlro, &path("review"), &[]).await.assert_status(422);
}

/// DW5: a partner, a manager, an accountant and a client user cannot see the
/// case: 404 on the list, the case and its actions, and its facts are on no
/// page they can open (`ACC.ACL.05`).
#[renox::test]
async fn staff_without_the_mlro_permission_cannot_see_the_case() {
    let t = boot_real().await;
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let (client, engagement) = falcon(&t, true).await;
    confirmed(&t, client.ubo, &mlro, &deputy).await;
    let case = t.aml_cases().await.remove(0);
    let path = format!("/accounting/aml-cases/{}", case.id);

    let partner = t.partner().await;
    let manager = t.manager().await;
    let accountant = t.accountant().await;
    let owner = t.portal_user("Hana", client.director, client.company).await;
    for viewer in [&partner, &manager, &accountant, &owner] {
        t.get(viewer, "/accounting/aml-cases")
            .await
            .assert_not_found();
        t.get(viewer, &path).await.assert_not_found();
        for action in ["review", "close", "report"] {
            t.post(viewer, &format!("{path}/{action}"), &[("reason", "x")])
                .await
                .assert_not_found();
        }
    }
    assert_eq!(t.aml_case(&case).await.status, AmlStatus::Open);

    // The MLRO and the deputy see it.
    for viewer in [&mlro, &deputy] {
        t.get(viewer, "/accounting/aml-cases")
            .await
            .assert_ok()
            .assert_see(&case.number());
        t.get(viewer, &path)
            .await
            .assert_ok()
            .assert_see("Omar K.")
            .assert_see("QDi.001");
    }

    // Nothing of it shows on the pages the others can open.
    for viewer in [&partner, &manager] {
        for page in [
            format!("/accounting/engagements/{}", engagement.id),
            "/accounting/engagements".to_owned(),
        ] {
            let text = t.get(viewer, &page).await.text();
            for hidden in ["AML-", "anction", "Omar K.", "QDi.001"] {
                assert!(!text.contains(hidden), "{page} shows `{hidden}`");
            }
        }
    }
}

/// A member of staff raises a concern: the MLRO and deputy are told, the
/// caller sees only that the MLRO is told, and the case is the MLRO's.
#[renox::test]
async fn staff_raise_a_concern_and_see_nothing_of_the_case() {
    let t = boot_real().await;
    let (mlro, deputy) = (t.mlro().await, t.deputy().await);
    let (client, engagement) = falcon(&t, true).await;
    let manager = t.manager().await;
    let portal = t.portal_user("Hana", client.director, client.company).await;
    let path = format!("/accounting/engagements/{}/aml-concern", engagement.id);

    // The facts are required; the client has no way in.
    t.post(&manager, &path, &[("facts", " ")])
        .await
        .assert_status(422);
    t.post(&portal, &path, &[("facts", "x")])
        .await
        .assert_forbidden();
    assert!(t.aml_cases().await.is_empty());

    let answer = t
        .post(
            &manager,
            &path,
            &[("facts", "Cash deposits that do not fit")],
        )
        .await;
    answer.assert_ok();
    assert!(!answer.text().contains("AML-"), "{}", answer.text());
    let cases = t.aml_cases().await;
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].kind, AmlKind::Concern);
    assert_eq!(cases[0].status, AmlStatus::Open);
    assert_eq!(cases[0].raised_by, Some(manager.id));
    assert_eq!(cases[0].client(), client.company);
    let told: Vec<_> = t
        .notices
        .of_type(CASE_NOTICE)
        .iter()
        .flat_map(|n| n.recipients.clone())
        .collect();
    assert!(told.contains(&NotifyRecipient::User { id: mlro.id }));
    assert!(told.contains(&NotifyRecipient::User { id: deputy.id }));
    assert_eq!(told.len(), 2);

    // The one who raised it does not see it; the engagement is untouched.
    t.get(&manager, &format!("/accounting/aml-cases/{}", cases[0].id))
        .await
        .assert_not_found();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    assert_eq!(
        t.party_row(client.company).await.status,
        PartyStatus::Active
    );
    // A concern needs no tasks to close, but it must be reviewed first.
    t.post(
        &mlro,
        &format!("/accounting/aml-cases/{}/review", cases[0].id),
        &[],
    )
    .await
    .assert_ok();
    t.post(
        &mlro,
        &format!("/accounting/aml-cases/{}/close", cases[0].id),
        &[("reason", "Explained by the seasonal trade")],
    )
    .await
    .assert_ok();
    assert_eq!(t.aml_cases().await[0].status, AmlStatus::ClosedNoAction);
}

/// `ACC.RUL.12` is compliance-critical: a run that fails on all five
/// attempts alerts the partner role and the company admin, once.
#[renox::test]
async fn a_failed_run_of_the_rule_alerts_the_partner_and_the_admin() {
    use renox::auth::permissions;
    use wl_kernel::COMPANY_ADMIN_ROLE;

    let t = boot_real().await;
    let (client, _engagement) = falcon(&t, true).await;
    let partner = t.partner().await;
    permissions::define_role(t.db(), COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let admin = t.user_with("Ada", COMPANY_ADMIN_ROLE).await;
    renox::db::sql(
        "ALTER TABLE \"accounting_aml_matches\" RENAME TO \"accounting_aml_matches_off\"",
    )
    .execute(t.db())
    .await
    .unwrap();
    let event = ScreeningMatchConfirmed {
        party: client.ubo,
        screening: ScreeningId(77),
    };
    let alerted = |user: i64| {
        t.notices.all().iter().any(|r| {
            r.type_key == "kernel.compliance_alert"
                && r.recipients.contains(&NotifyRecipient::User { id: user })
        })
    };
    for wait in [60_u64, 300, 900, 3600] {
        let outcome = t
            .run(aml_cases::on_match_confirmed(t.state(), &event))
            .await;
        assert_eq!(outcome.unwrap(), RunOutcome::Retrying);
        assert!(!alerted(partner.id), "not before the last attempt");
        t.app.travel(Duration::from_secs(wait));
    }
    let last = t
        .run(aml_cases::on_match_confirmed(t.state(), &event))
        .await;
    assert_eq!(last.unwrap(), RunOutcome::Failed);
    let alerts: Vec<ComplianceAlert> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].owner_role, support::PARTNER);
    assert!(alerted(partner.id), "the partner role is told");
    assert!(alerted(admin.id), "the company admin is told");
}
