//! BL-CON-11 (#290): a rejected variation is no longer an end. The cost of
//! work executed on a site instruction stays on the variation's cost code,
//! marked unrecovered; the project manager resubmits it as a new revision,
//! turns it into a claim for the final account (settled at an agreed amount by
//! the QS) or abandons it with the MD's approval (`CON.ACL.16`).

mod support;

use construction::variation::{self, Variation};
use construction::{Project, ProjectStatus, VariationStatus};
use renox::prelude::*;

use support::{Booted, boot, users_of};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

fn path(variation: &Variation, step: &str) -> String {
    format!("/construction/variations/{}/{step}", variation.id)
}

/// A project in progress with a contract value of AED 2,000,000.
async fn project(t: &Booted, manager: &User) -> Project {
    let project = t.project(ProjectStatus::InProgress, Some(manager)).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("contract_value", &aed(2_000_000))])
        .await
        .unwrap();
    t.reload(&project).await
}

/// The QS records a variation executed on site instruction SI-22 priced AED
/// 50,000 and the project manager submits it; AED 40,000 of cost is booked on
/// its cost code.
async fn submitted(
    t: &Booted,
    qs: &User,
    pm: &User,
    project: &Project,
    executed: bool,
) -> Variation {
    let mut form = vec![
        ("kind", "addition"),
        ("description", "Extra partition on level 3"),
        ("site_instruction", "SI-22"),
        ("priced_amount", "5000000"),
    ];
    if executed {
        form.push(("executed", "on"));
    }
    t.post(
        qs,
        &format!("/construction/projects/{}/variations", project.id),
        &form,
    )
    .await
    .assert_ok();
    let made = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .expect("the variation");
    t.post(pm, &path(&made, "submit"), &[]).await.assert_ok();
    if executed {
        variation::book_cost(
            t.state(),
            made.id,
            aed(40_000),
            &format!("line-{}", made.id),
        )
        .await
        .unwrap();
    }
    t.reload_variation(made.id).await
}

/// The client rejects `made`: the QS records it with the reason.
async fn reject_it(t: &Booted, qs: &User, made: &Variation) -> Variation {
    t.post(
        qs,
        &path(made, "reject"),
        &[("reason", "not instructed by employer")],
    )
    .await
    .assert_ok();
    t.reload_variation(made.id).await
}

/// Done when 1: VO executed on site instruction SI-22 with cost AED 40,000
/// booked, `reject` with reason "not instructed by employer" makes it
/// `rejected`, marks AED 40,000 unrecovered and notifies the QS and project
/// manager.
#[renox::test]
async fn rejecting_an_executed_variation_marks_its_cost_unrecovered() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let made = submitted(&t, &qs, &pm, &project, true).await;
    assert_eq!(made.status, VariationStatus::Submitted);
    assert_eq!(made.code, "VO-01");
    assert_eq!(made.cost_booked, aed(40_000));
    assert_eq!(made.unrecovered_amount, 0, "nothing is unrecovered yet");
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].cost_code.as_deref(), Some("vo-01"));
    assert_eq!(posted[0].amount_aed, aed(40_000));

    // The reason is needed, and only the QS records the client's decision.
    t.post(&qs, &path(&made, "reject"), &[("reason", " ")])
        .await
        .assert_status(422);
    t.post(&pm, &path(&made, "reject"), &[("reason", "no")])
        .await
        .assert_status(403);
    assert_eq!(
        t.reload_variation(made.id).await.status,
        VariationStatus::Submitted
    );

    let now = reject_it(&t, &qs, &made).await;
    assert_eq!(now.status, VariationStatus::Rejected);
    assert_eq!(
        now.rejection_reason.as_deref(),
        Some("not instructed by employer")
    );
    assert_eq!(now.unrecovered_amount, aed(40_000));
    assert_eq!(
        now.cost_booked,
        aed(40_000),
        "the cost stays on its cost code"
    );
    assert_eq!(
        t.costs.posted().len(),
        1,
        "nothing is moved to another cost code"
    );
    let told = t.notices.notices_titled("rejected");
    assert_eq!(told.len(), 1);
    let mut users = users_of(&told[0]);
    users.sort_unstable();
    let mut wanted = vec![qs.id, pm.id];
    wanted.sort_unstable();
    assert_eq!(users, wanted);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("not instructed by employer")
    );
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("AED 40,000.00")
    );
    let report = variation::unrecovered_of(t.db(), project.id).await.unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].unrecovered_amount, aed(40_000));

    // Further cost booked after the rejection counts as unrecovered too.
    variation::book_cost(t.state(), made.id, aed(5_000), "line-2")
        .await
        .unwrap();
    assert_eq!(
        t.reload_variation(made.id).await.unrecovered_amount,
        aed(45_000)
    );

    // A variation that was not executed has no unrecovered cost.
    let idle = submitted(&t, &qs, &pm, &project, false).await;
    let idle = reject_it(&t, &qs, &idle).await;
    assert_eq!(idle.unrecovered_amount, 0);
}

/// Done when 2: `resubmit` creates revision 2 in `draft`; revision 1 stays
/// `rejected` and read-only, "resubmitted as rev 2", and can be neither
/// claimed nor abandoned nor resubmitted again.
#[renox::test]
async fn resubmitting_makes_a_new_revision_and_keeps_the_rejected_one() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let made = submitted(&t, &qs, &pm, &project, true).await;
    let first = reject_it(&t, &qs, &made).await;

    // Only the project manager resubmits.
    t.post(&qs, &path(&first, "resubmit"), &[])
        .await
        .assert_status(403);
    t.post(&pm, &path(&first, "resubmit"), &[])
        .await
        .assert_ok();
    let revisions = variation::revisions_of(t.db(), project.id, &first.code)
        .await
        .unwrap();
    assert_eq!(revisions.len(), 2);
    let (old, new) = (&revisions[0], &revisions[1]);
    assert_eq!(old.status, VariationStatus::Rejected);
    assert_eq!(old.resubmitted_as, Some(2));
    assert_eq!(
        old.resubmitted_text().as_deref(),
        Some("resubmitted as rev 2")
    );
    assert_eq!(new.revision, 2);
    assert_eq!(new.status, VariationStatus::Draft);
    assert_eq!(new.code, old.code);
    assert_eq!(new.cost_code, old.cost_code);
    assert_eq!(new.description, old.description);
    assert_eq!(new.priced_amount, old.priced_amount);
    assert_eq!(new.cost_booked, aed(40_000));
    assert_eq!(new.unrecovered_amount, 0);
    assert_eq!(new.rejection_reason, None);
    assert_eq!(new.resubmitted_as, None);

    // The rejected revision can no longer be resubmitted, claimed or
    // abandoned, and does not show in the unrecovered report.
    for step in ["resubmit", "claim", "abandon"] {
        t.post(&pm, &path(old, step), &[]).await.assert_status(422);
    }
    assert!(!t.has_pending(&old.subject()).await);
    assert!(
        variation::unrecovered_of(t.db(), project.id)
            .await
            .unwrap()
            .is_empty()
    );

    // The new revision is priced again and submitted on its own clock.
    t.post(
        &qs,
        &path(new, "price"),
        &[
            ("kind", "addition"),
            (
                "description",
                "Extra partition on level 3, with the drawings",
            ),
            ("site_instruction", "SI-22"),
            ("executed", "on"),
            ("priced_amount", "4500000"),
        ],
    )
    .await
    .assert_ok();
    let repriced = t.reload_variation(new.id).await;
    assert_eq!(repriced.priced_amount, aed(45_000));
    t.post(&pm, &path(&repriced, "submit"), &[])
        .await
        .assert_ok();
    assert_eq!(
        t.reload_variation(new.id).await.status,
        VariationStatus::Submitted
    );
    // A submitted revision is not a draft: it cannot be priced again.
    t.post(
        &qs,
        &path(&repriced, "price"),
        &[
            ("kind", "addition"),
            ("description", "again"),
            ("priced_amount", "1"),
        ],
    )
    .await
    .assert_status(422);
}

/// Done when 3: `claim` lists the variation in the project's claims;
/// `settle_claim` at AED 32,000 with the client's reference makes it
/// `approved` and adds AED 32,000 to the contract value, the budget and the
/// next IPA. `claim` on a variation not executed is refused.
#[renox::test]
async fn a_claim_is_settled_at_the_agreed_amount() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let made = submitted(&t, &qs, &pm, &project, true).await;
    let rejected = reject_it(&t, &qs, &made).await;

    // Only the project manager claims.
    t.post(&qs, &path(&rejected, "claim"), &[])
        .await
        .assert_status(403);
    t.post(&pm, &path(&rejected, "claim"), &[])
        .await
        .assert_ok();
    let claimed = t.reload_variation(made.id).await;
    assert_eq!(claimed.status, VariationStatus::Claimed);
    let claims = variation::claims_of(t.db(), project.id).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, made.id);
    // It shows in the claims of the final account and in the unrecovered
    // report until it is settled.
    let closing = t
        .get(
            &pm,
            &format!("/construction/projects/{}/closing", project.id),
        )
        .await;
    closing.assert_ok();
    let view: renox::serde_json::Value = renox::serde_json::from_str(&closing.text()).unwrap();
    assert_eq!(view["variation_claims"][0]["code"], "VO-01");
    assert_eq!(
        view["unrecovered_variations"][0]["unrecovered_amount"],
        aed(40_000)
    );

    // The client's reference and the agreed amount are needed; only the QS
    // records them.
    let settle = path(&claimed, "settle-claim");
    t.post(
        &qs,
        &settle,
        &[("client_reference", " "), ("approved_amount", "3200000")],
    )
    .await
    .assert_status(422);
    t.post(&qs, &settle, &[("client_reference", "CL-9")])
        .await
        .assert_status(422);
    t.post(
        &pm,
        &settle,
        &[("client_reference", "CL-9"), ("approved_amount", "3200000")],
    )
    .await
    .assert_status(403);
    assert_eq!(
        t.reload_variation(made.id).await.status,
        VariationStatus::Claimed
    );
    let before = t.reload(&project).await;

    t.post(
        &qs,
        &settle,
        &[("client_reference", "CL-9"), ("approved_amount", "3200000")],
    )
    .await
    .assert_ok();
    let settled = t.reload_variation(made.id).await;
    assert_eq!(settled.status, VariationStatus::Approved);
    assert_eq!(settled.approved_amount, Some(aed(32_000)));
    assert_eq!(settled.client_reference.as_deref(), Some("CL-9"));
    let after = t.reload(&project).await;
    assert_eq!(after.contract_value, before.contract_value + aed(32_000));
    assert_eq!(after.budget, before.budget + aed(32_000));
    assert_eq!(
        variation::next_ipa_amount(t.db(), project.id)
            .await
            .unwrap(),
        aed(32_000)
    );
    assert!(
        variation::claims_of(t.db(), project.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        variation::unrecovered_of(t.db(), project.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Done when 5: `claim` on a variation not executed is refused, `resubmit`
/// and `abandon` remain.
#[renox::test]
async fn a_variation_not_executed_cannot_be_claimed() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let made = submitted(&t, &qs, &pm, &project, false).await;
    let idle = reject_it(&t, &qs, &made).await;
    let refused = t.post(&pm, &path(&idle, "claim"), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("executed"), "{}", refused.text());
    assert_eq!(
        t.reload_variation(made.id).await.status,
        VariationStatus::Rejected
    );
    // Abandoning still needs the MD, whatever the amount.
    t.post(&pm, &path(&idle, "abandon"), &[]).await.assert_ok();
    assert!(t.has_pending(&idle.subject()).await);
    assert_eq!(
        t.reload_variation(made.id).await.status,
        VariationStatus::Rejected
    );
}

/// Done when 4: `abandon` requests MD approval; once approved it is
/// `abandoned` and AED 40,000 is in the unrecovered variations report. A claim
/// can be abandoned too.
#[renox::test]
async fn abandoning_needs_the_md_and_writes_the_cost_into_the_unrecovered_report() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let md = t.md("Omar").await;
    let project = project(&t, &pm).await;
    let made = submitted(&t, &qs, &pm, &project, true).await;
    let rejected = reject_it(&t, &qs, &made).await;

    // The QS may not abandon.
    t.post(&qs, &path(&rejected, "abandon"), &[])
        .await
        .assert_status(403);
    t.post(&pm, &path(&rejected, "abandon"), &[])
        .await
        .assert_ok();
    // Nothing changed until the MD decides.
    let waiting = t.reload_variation(made.id).await;
    assert_eq!(waiting.status, VariationStatus::Rejected);
    assert!(t.has_pending(&waiting.subject()).await);
    let request = t.pending_about(&waiting.subject()).await;
    assert_eq!(request.requester_id, pm.id);
    // The requester cannot approve.
    let own = wl_kernel::approvals::approve(t.state(), request.id, &pm, "").await;
    assert!(own.is_err());

    t.approve_about(&md, &waiting.subject()).await;
    let gone = t.reload_variation(made.id).await;
    assert_eq!(gone.status, VariationStatus::Abandoned);
    assert_eq!(gone.unrecovered_amount, aed(40_000));
    let report = variation::unrecovered_of(t.db(), project.id).await.unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].id, made.id);
    assert_eq!(report[0].unrecovered_amount, aed(40_000));
    assert_eq!(
        t.reload(&project).await.contract_value,
        aed(2_000_000),
        "an abandoned variation changes no value"
    );

    // A claim that is not settled can be abandoned too.
    let second = submitted(&t, &qs, &pm, &project, true).await;
    let second = {
        let r = reject_it(&t, &qs, &second).await;
        t.post(&pm, &path(&r, "claim"), &[]).await.assert_ok();
        t.reload_variation(r.id).await
    };
    assert_eq!(second.status, VariationStatus::Claimed);
    t.post(&pm, &path(&second, "abandon"), &[])
        .await
        .assert_ok();
    t.approve_about(&md, &second.subject()).await;
    assert_eq!(
        t.reload_variation(second.id).await.status,
        VariationStatus::Abandoned
    );
    assert_eq!(
        variation::unrecovered_of(t.db(), project.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

/// A draft is priced by the QS and submitted by the project manager only
/// (`CON.ACL.06`); a draft or a submitted variation can be withdrawn.
#[renox::test]
async fn the_qs_prices_and_only_the_project_manager_submits_or_withdraws() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let create = format!("/construction/projects/{}/variations", project.id);
    let form = [
        ("kind", "addition"),
        ("description", "Extra door"),
        ("priced_amount", "100000"),
    ];
    t.post(&pm, &create, &form).await.assert_status(403);
    t.post(&qs, &create, &[("kind", "addition"), ("description", " ")])
        .await
        .assert_status(422);
    t.post(&qs, &create, &[("kind", "wall"), ("description", "x")])
        .await
        .assert_status(422);
    t.post(&qs, &create, &form).await.assert_ok();
    let made = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(made.status, VariationStatus::Draft);
    assert_eq!(made.code, "VO-01");
    assert_eq!(made.cost_code, "vo-01");

    t.post(&qs, &path(&made, "submit"), &[])
        .await
        .assert_status(403);
    // Unpriced, it is not submitted.
    t.post(
        &qs,
        &create,
        &[("kind", "addition"), ("description", "Unpriced")],
    )
    .await
    .assert_ok();
    let unpriced = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(unpriced.code, "VO-02");
    t.post(&pm, &path(&unpriced, "submit"), &[])
        .await
        .assert_status(422);

    t.post(&pm, &path(&made, "submit"), &[]).await.assert_ok();
    let sent = t.reload_variation(made.id).await;
    assert_eq!(sent.status, VariationStatus::Submitted);
    assert!(sent.submitted_on.is_some());
    assert_eq!(sent.submitted_on, sent.notice_date);
    t.post(&qs, &path(&sent, "withdraw"), &[])
        .await
        .assert_status(403);
    t.post(&pm, &path(&sent, "withdraw"), &[]).await.assert_ok();
    assert_eq!(
        t.reload_variation(made.id).await.status,
        VariationStatus::Withdrawn
    );
    t.post(&pm, &path(&unpriced, "withdraw"), &[])
        .await
        .assert_ok();
}

/// BL-CON-03 (#282) done when 2: the running project of a blocked client goes
/// on with its variations too; the QS records the approval of a submitted
/// variation of AED 50,000 and it adds to the contract value, no exception or
/// review being asked.
#[renox::test]
async fn a_variation_of_a_blocked_clients_running_project_is_approved() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let made = submitted(&t, &qs, &pm, &project, false).await;
    t.blocked.block(support::CLIENT_PARTY);
    let before = t.reload(&project).await;

    t.post(
        &qs,
        &path(&made, "approve"),
        &[
            ("client_reference", "CL-50"),
            ("approved_amount", "5000000"),
        ],
    )
    .await
    .assert_ok();

    let approved = t.reload_variation(made.id).await;
    assert_eq!(approved.status, VariationStatus::Approved);
    assert_eq!(approved.approved_amount, Some(aed(50_000)));
    let after = t.reload(&project).await;
    assert_eq!(after.status, ProjectStatus::InProgress);
    assert_eq!(after.contract_value, before.contract_value + aed(50_000));
    assert!(!t.has_pending(&approved.subject()).await);
    assert!(!t.has_pending(&project.subject()).await);
}
