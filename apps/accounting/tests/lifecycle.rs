//! The base of the 04 Engagement and CDD file lifecycles (base for #325,
//! #331, #332): activation gathers every failing check in one refusal; a
//! ready engagement activates and, after the commit, gets its compliance
//! calendar and its retainer; decline, suspend and end take neutral reason
//! codes only; a rejected CDD file declines the onboarding engagements.

mod support;

use accounting::calendar::{CT_RETURN, VAT_RETURN};
use accounting::engagement_machine::AUTO_DECLINE;
use accounting::{CddStatus, EngagementStatus};
use renox::prelude::*;
use renox::state_machine::history;
use wl_kernel::deadlines::TaskRecurrence;
use wl_kernel::documents::today;

use support::boot;

#[renox::test]
async fn activation_lists_every_failing_check_in_one_refusal() {
    let t = boot().await;
    let client = t.client().await;
    let engagement = t.engagement(&client).await;
    let manager = t.manager().await;

    let res = t.activate(&manager, &engagement).await;
    res.assert_status(422);
    let text = res.text();
    assert!(
        text.contains("This engagement cannot be activated:"),
        "{text}"
    );
    assert!(text.contains("The CDD file is not approved"), "{text}");
    assert!(
        text.contains("The engagement letter is not signed"),
        "{text}"
    );
    assert!(text.contains("No tax profile"), "{text}");
    assert_eq!(t.status(&engagement).await, EngagementStatus::Onboarding);
    assert!(
        history(t.db(), accounting::Engagement::TABLE, engagement.id)
            .await
            .unwrap()
            .is_empty()
    );

    // Without `engagements.manage` the answer is 403, whatever is missing.
    let accountant = t.accountant().await;
    t.activate(&accountant, &engagement)
        .await
        .assert_forbidden();
}

#[renox::test]
async fn a_ready_engagement_activates_with_its_calendar_and_its_retainer() {
    let t = boot().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let engagement = t.ready_engagement(&client, &mlro).await;
    let manager = t.manager().await;

    t.activate(&manager, &engagement).await.assert_ok();
    let active = t.reload(&engagement).await;
    assert_eq!(active.status, EngagementStatus::Active);
    assert_eq!(active.activated_on, Some(today()));
    assert_eq!(active.start_date, Some(today()));

    // After the commit: one VAT return and one corporate tax return.
    let tasks = t.tasks.of(&active.subject());
    let types: Vec<&str> = tasks.iter().map(|task| task.task_type.as_str()).collect();
    assert_eq!(types, [VAT_RETURN, CT_RETURN]);
    assert_eq!(tasks[0].recurrence, TaskRecurrence::Quarterly);
    assert_eq!(tasks[0].party, Some(client.company));
    assert_eq!(
        tasks[0].owner_role.as_deref(),
        Some(accounting::permissions::ACCOUNTANT_ROLE)
    );
    assert!(tasks[0].evidence_required);
    assert_eq!(tasks[1].recurrence, TaskRecurrence::Yearly);

    // And the retainer: AED 5,000 a month from today, billed to the client.
    let started = t.subscriptions.all();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].party, client.company);
    assert_eq!(started[0].source, active.subject());
    assert_eq!(started[0].unit_price, 500_000);
    assert_eq!(started[0].start, today());
}

#[renox::test]
async fn reasons_are_neutral_codes_and_a_restricted_one_is_refused() {
    let t = boot().await;
    let client = t.client().await;
    let manager = t.manager().await;
    let partner = t.partner().await;

    let onboarding = t.engagement(&client).await;
    t.act(&manager, &onboarding, "decline", &[("reason", "sanctions")])
        .await
        .assert_status(422);
    t.act(
        &manager,
        &onboarding,
        "decline",
        &[("reason", "client_withdrew")],
    )
    .await
    .assert_ok();
    let declined = t.reload(&onboarding).await;
    assert_eq!(declined.status, EngagementStatus::Declined);
    assert_eq!(declined.reason_code.as_deref(), Some("client_withdrew"));

    let active = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    for restricted in ["sanctions", "aml_concern", ""] {
        t.act(&manager, &active, "suspend", &[("reason", restricted)])
            .await
            .assert_status(422);
    }
    t.act(&manager, &active, "suspend", &[("reason", "non_payment")])
        .await
        .assert_ok();
    assert_eq!(t.status(&active).await, EngagementStatus::Suspended);
    t.act(&manager, &active, "resume", &[]).await.assert_ok();
    assert_eq!(t.status(&active).await, EngagementStatus::Active);

    // Only the partner ends an engagement.
    t.act(&manager, &active, "end", &[("reason", "client_request")])
        .await
        .assert_forbidden();
    t.act(&partner, &active, "end", &[("reason", "client_request")])
        .await
        .assert_ok();
    let ended = t.reload(&active).await;
    assert_eq!(ended.status, EngagementStatus::Ended);
    assert_eq!(ended.ended_on, Some(today()));
    assert_eq!(ended.reason_code.as_deref(), Some("client_request"));
    assert!(ended.may_message_client());
}

#[renox::test]
async fn submit_needs_ubos_documents_and_a_rating_and_a_rejection_declines_onboarding() {
    let t = boot().await;
    let client = t.client().await;
    let accountant = t.accountant().await;
    let mlro = t.mlro().await;

    // A bare file: no UBO, no documents, no rating.
    let bare = accounting::cdd_files::open(t.db(), None, client.company)
        .await
        .unwrap();
    let res = t
        .post(
            &accountant,
            &format!("/accounting/cdd/{}/submit", bare.id),
            &[],
        )
        .await;
    res.assert_status(422);
    let text = res.text();
    assert!(text.contains("No UBO is identified"), "{text}");
    assert!(
        text.contains("The identity documents are not attached"),
        "{text}"
    );
    assert!(text.contains("The risk is not rated"), "{text}");

    // A prepared file is submitted and rejected by the MLRO: the onboarding
    // engagement is declined as not accepted, and the reason stays on the
    // file for the MLRO.
    let other = t.client_named(" B").await;
    let file = t.cdd_prepared(&other).await;
    t.lists_current().await;
    t.screen_all_clear(&other).await;
    let engagement = t.engagement(&other).await;
    t.post(
        &accountant,
        &format!("/accounting/cdd/{}/submit", file.id),
        &[],
    )
    .await
    .assert_ok();
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/reject", file.id),
        &[("reason", "source of funds not explained")],
    )
    .await
    .assert_ok();
    let rejected = t.cdd(&file).await;
    assert_eq!(rejected.status, CddStatus::Rejected);
    assert_eq!(
        rejected.decision_reason.as_deref(),
        Some("source of funds not explained")
    );
    let declined = t.reload(&engagement).await;
    assert_eq!(declined.status, EngagementStatus::Declined);
    assert_eq!(declined.reason_code.as_deref(), Some("not_accepted"));
    assert!(!declined.may_message_client());
    let rows = history(t.db(), accounting::Engagement::TABLE, engagement.id)
        .await
        .unwrap();
    assert_eq!(rows.last().unwrap().transition, AUTO_DECLINE);
    // The CDD history never holds the reason.
    let cdd_rows = history(t.db(), accounting::CddFile::TABLE, file.id)
        .await
        .unwrap();
    assert!(cdd_rows.iter().all(|row| row.reason.is_none()));
    // The preparer does not see it on the page; the MLRO does.
    t.get(&accountant, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_ok()
        .assert_dont_see("source of funds");
    t.get(&mlro, &format!("/accounting/cdd/{}", file.id))
        .await
        .assert_ok()
        .assert_see("source of funds not explained");
}
