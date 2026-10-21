//! BL-CON-08 (#287): the retention the client keeps (`deduct`, `CON.ACL.14`).
//! The QS records what the client keeps of a `held` or `due` part with the
//! client's notice; the project manager accepts it (the MD above AED 25,000)
//! or contests it; only an accepted deduction reduces the part, books the
//! amount to the back-charge cost code and issues no tax document; a part
//! with nothing left is `kept`. After invoicing the retention invoice is
//! disputed instead. The cost and the invoices are replaced by fakes that
//! record what the vertical asks for.

mod support;

use construction::retention_deduction::{self, BACK_CHARGE_COST_CODE, RetentionDeduction};
use construction::{
    DeductionStatus, Project, ProjectStatus, Retention, RetentionPart, RetentionStatus,
};
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::invoicing::DisputeBy;

use support::{Booted, boot};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

fn record_path(part: &Retention) -> String {
    format!("/construction/retentions/{}/deductions", part.id)
}

fn accept_path(line: &RetentionDeduction) -> String {
    format!("/construction/retention-deductions/{}/accept", line.id)
}

fn contest_path(line: &RetentionDeduction) -> String {
    format!("/construction/retention-deductions/{}/contest", line.id)
}

/// The deduction `amount` fils the QS records on `part`.
async fn record(t: &Booted, by: &User, part: &Retention, amount: i64) -> RetentionDeduction {
    t.post(
        by,
        &record_path(part),
        &[
            ("amount", &amount.to_string()),
            ("reference", "CLIENT-NOTICE-9"),
            ("reason", "Defects the client fixed itself"),
        ],
    )
    .await
    .assert_ok();
    retention_deduction::lines_of(t.db(), part.id)
        .await
        .unwrap()
        .pop()
        .expect("the deduction")
}

async fn reload_line(t: &Booted, id: i64) -> RetentionDeduction {
    RetentionDeduction::find_or_404(t.db(), id).await.unwrap()
}

/// A running project with the final part in `status` holding AED 50,000.
async fn with_final_part(
    t: &Booted,
    manager: &User,
    status: RetentionStatus,
) -> (Project, Retention) {
    let project = t
        .project(ProjectStatus::PracticalCompletion, Some(manager))
        .await;
    t.retention(
        &project,
        RetentionPart::First,
        aed(50_000),
        RetentionStatus::Invoiced,
    )
    .await;
    let part = t
        .retention(&project, RetentionPart::Final, aed(50_000), status)
        .await;
    (project, part)
}

/// Done when 1 and 2: the final part `held` at AED 50,000; a deduction of
/// AED 8,000 with the client's notice, accepted by the project manager,
/// leaves AED 42,000 `held` and books AED 8,000 to the back-charge cost code;
/// a deduction of AED 42,000 accepted by the MD (above AED 25,000) makes the
/// part `kept`. No tax document exists.
#[renox::test]
async fn an_accepted_deduction_reduces_the_part_and_the_md_accepts_the_rest() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let md = t.md("Bader").await;
    let (project, part) = with_final_part(&t, &pm, RetentionStatus::Held).await;

    let first = record(&t, &qs, &part, aed(8_000)).await;
    assert_eq!(first.status, DeductionStatus::Undecided);
    assert_eq!(first.recorded_by, qs.id);
    assert_eq!(first.reference, "CLIENT-NOTICE-9");
    // Not applied before it is accepted.
    assert_eq!(t.reload_part(part.id).await.amount, aed(50_000));
    assert!(t.costs.posted().is_empty());

    t.post(&pm, &accept_path(&first), &[]).await.assert_ok();

    let reduced = t.reload_part(part.id).await;
    assert_eq!(reduced.status, RetentionStatus::Held);
    assert_eq!(reduced.amount, aed(42_000));
    let accepted = reload_line(&t, first.id).await;
    assert_eq!(accepted.status, DeductionStatus::Accepted);
    assert_eq!(accepted.decided_by, Some(pm.id));
    assert!(accepted.booked_at.is_some());
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, aed(8_000));
    assert_eq!(posted[0].cost_code.as_deref(), Some(BACK_CHARGE_COST_CODE));
    assert_eq!(posted[0].cost_object, project.subject());
    assert_eq!(posted[0].source, first.subject());
    assert!(t.invoices.drafts().is_empty(), "no tax document");
    assert!(t.invoices.credits().is_empty(), "no credit note");

    // The rest is above AED 25,000: the MD decides, and the part is kept.
    let rest = record(&t, &qs, &reduced, aed(42_000)).await;
    t.post(&pm, &accept_path(&rest), &[]).await.assert_ok();
    assert_eq!(
        reload_line(&t, rest.id).await.status,
        DeductionStatus::Undecided,
        "waits for the MD"
    );
    assert_eq!(t.reload_part(part.id).await.amount, aed(42_000));
    assert_eq!(t.costs.posted().len(), 1);
    let request = ApprovalRequest::pending_about(t.db(), &rest.subject())
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("an approval request");
    assert_eq!(request.requester_id, pm.id);
    let refused = approve(t.state(), request.id, &pm, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");

    approve(t.state(), request.id, &md, "").await.unwrap();

    let kept = t.reload_part(part.id).await;
    assert_eq!(kept.status, RetentionStatus::Kept);
    assert_eq!(kept.amount, 0);
    assert_eq!(
        reload_line(&t, rest.id).await.status,
        DeductionStatus::Accepted
    );
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[1].amount_aed, aed(42_000));
    assert!(t.invoices.drafts().is_empty());

    // A kept part takes no more.
    t.post(
        &qs,
        &record_path(&kept),
        &[("amount", "100"), ("reference", "N-1"), ("reason", "More")],
    )
    .await
    .assert_status(422);
}

/// A part that is `due` stays `due` while an amount remains; the time it
/// became due is kept.
#[renox::test]
async fn a_due_part_stays_due_when_an_amount_remains() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let (_project, part) = with_final_part(&t, &pm, RetentionStatus::Due).await;

    let line = record(&t, &qs, &part, aed(10_000)).await;
    t.post(&pm, &accept_path(&line), &[]).await.assert_ok();

    let after = t.reload_part(part.id).await;
    assert_eq!(after.status, RetentionStatus::Due);
    assert_eq!(after.amount, aed(40_000));
}

/// A contested deduction is not applied and not booked; it is listed with the
/// items of the final account.
#[renox::test]
async fn a_contested_deduction_changes_nothing_and_is_listed_for_the_final_account() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let (project, part) = with_final_part(&t, &pm, RetentionStatus::Held).await;

    let line = record(&t, &qs, &part, aed(8_000)).await;
    // The QS records, the project manager decides.
    t.post(&qs, &contest_path(&line), &[])
        .await
        .assert_status(403);
    t.post(&pm, &contest_path(&line), &[]).await.assert_ok();

    assert_eq!(
        reload_line(&t, line.id).await.status,
        DeductionStatus::Contested
    );
    assert_eq!(t.reload_part(part.id).await.amount, aed(50_000));
    assert!(t.costs.posted().is_empty());
    // It is decided: it cannot be accepted afterwards.
    t.post(&pm, &accept_path(&line), &[])
        .await
        .assert_status(422);

    let closing = t
        .get(
            &pm,
            &format!("/construction/projects/{}/closing", project.id),
        )
        .await;
    closing.assert_ok();
    closing.assert_json_path("contested_retention_deductions.0.amount", aed(8_000));
    closing.assert_json_path(
        "contested_retention_deductions.0.reference",
        "CLIENT-NOTICE-9",
    );
}

/// Done when 3: on an `invoiced` part `deduct` is not offered; the user opens
/// the dispute of the retention invoice instead, for the amount and its VAT
/// (AED 8,000 + 400), which resolves `credited` as a credit note
/// (wl-invoicing, BL-INV-04).
#[renox::test]
async fn an_invoiced_part_is_disputed_instead_of_deducted() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let accountant = t.accountant("Aisha").await;
    let (_project, part) = with_final_part(&t, &pm, RetentionStatus::Due).await;
    t.post(
        &accountant,
        &format!("/construction/retentions/{}/invoice", part.id),
        &[],
    )
    .await
    .assert_ok();
    let invoiced = t.reload_part(part.id).await;
    assert_eq!(invoiced.status, RetentionStatus::Invoiced);

    let refused = t
        .post(
            &qs,
            &record_path(&invoiced),
            &[
                ("amount", &aed(8_000).to_string()),
                ("reference", "CLIENT-NOTICE-9"),
                ("reason", "Defects"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains("dispute"), "{}", refused.text());
    assert!(
        retention_deduction::lines_of(t.db(), part.id)
            .await
            .unwrap()
            .is_empty()
    );

    let path = format!("/construction/retentions/{}/dispute", part.id);
    t.post(&pm, &path, &[("amount", "800000"), ("reason", "Defects")])
        .await
        .assert_status(403);
    t.post(&qs, &path, &[("amount", "800000"), ("reason", " ")])
        .await
        .assert_status(422);
    t.post(
        &qs,
        &path,
        &[("amount", &aed(50_001).to_string()), ("reason", "Defects")],
    )
    .await
    .assert_status(422);
    t.post(&qs, &path, &[("amount", "800000"), ("reason", "Defects")])
        .await
        .assert_ok();

    let disputes = t.invoices.disputes();
    assert_eq!(disputes.len(), 1);
    assert_eq!(disputes[0].invoice.0, invoiced.invoice_id.unwrap());
    assert_eq!(disputes[0].amount, Some(aed(8_400)), "AED 8,000 + VAT 400");
    assert_eq!(disputes[0].by, DisputeBy::User { id: qs.id });
    assert_eq!(disputes[0].source, Some(invoiced.subject()));
    // The part is not reduced and no cost is booked: the credit note will be.
    assert_eq!(t.reload_part(part.id).await.amount, aed(50_000));
    assert!(t.costs.posted().is_empty());

    // A part that is not invoiced has no invoice to dispute.
    let (_other, held) = with_final_part(&t, &pm, RetentionStatus::Held).await;
    t.post(
        &qs,
        &format!("/construction/retentions/{}/dispute", held.id),
        &[("amount", "800000"), ("reason", "Defects")],
    )
    .await
    .assert_status(422);
}

/// A deduction needs the client's reference, a reason and an amount above 0
/// within the part; the QS records it and nobody else.
#[renox::test]
async fn a_deduction_needs_its_reference_its_reason_and_an_amount_within_the_part() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let (_project, part) = with_final_part(&t, &pm, RetentionStatus::Held).await;
    let path = record_path(&part);
    let amount = aed(8_000).to_string();

    let too_much = aed(50_001).to_string();
    let cases: Vec<Vec<(&str, &str)>> = vec![
        vec![("amount", amount.as_str()), ("reason", "Defects")],
        vec![
            ("amount", amount.as_str()),
            ("reference", " "),
            ("reason", "Defects"),
        ],
        vec![("amount", amount.as_str()), ("reference", "N-1")],
        vec![("amount", "0"), ("reference", "N-1"), ("reason", "D")],
        vec![("amount", "-5"), ("reference", "N-1"), ("reason", "D")],
        vec![("amount", "many"), ("reference", "N-1"), ("reason", "D")],
        vec![
            ("amount", too_much.as_str()),
            ("reference", "N-1"),
            ("reason", "D"),
        ],
    ];
    for fields in &cases {
        t.post(&qs, &path, fields).await.assert_status(422);
    }
    t.post(
        &pm,
        &path,
        &[
            ("amount", &amount),
            ("reference", "N-1"),
            ("reason", "Defects"),
        ],
    )
    .await
    .assert_status(403);
    assert!(
        retention_deduction::lines_of(t.db(), part.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The user who recorded a deduction never accepts it, and two deductions
/// that together exceed the part cannot both be accepted: the second is
/// refused with what remains.
#[renox::test]
async fn nobody_accepts_their_own_deduction_and_the_part_cannot_go_below_zero() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qadir").await;
    let (_project, part) = with_final_part(&t, &pm, RetentionStatus::Held).await;

    // A user who is both the QS and the project manager.
    let both = t.qs("Both").await;
    both.assign_role(t.db(), construction::permissions::PROJECT_MANAGER_ROLE)
        .await
        .unwrap();
    let own = record(&t, &both, &part, aed(1_000)).await;
    let refused = t.post(&both, &accept_path(&own), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("recorded"), "{}", refused.text());
    assert_eq!(t.reload_part(part.id).await.amount, aed(50_000));

    // Each is below AED 25,000, so the project manager decides; together they
    // are more than the part holds.
    let first = record(&t, &qs, &part, aed(24_000)).await;
    let second = record(&t, &qs, &part, aed(24_000)).await;
    let third = record(&t, &qs, &part, aed(24_000)).await;
    t.post(&pm, &accept_path(&first), &[]).await.assert_ok();
    t.post(&pm, &accept_path(&second), &[]).await.assert_ok();
    let refused = t.post(&pm, &accept_path(&third), &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("AED 2,000.00"),
        "{}",
        refused.text()
    );
    assert_eq!(
        reload_line(&t, third.id).await.status,
        DeductionStatus::Undecided
    );
    assert_eq!(t.reload_part(part.id).await.amount, aed(2_000));
    assert_eq!(
        t.costs.posted().len(),
        2,
        "only what was accepted is booked"
    );
}
