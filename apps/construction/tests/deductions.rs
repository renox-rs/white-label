//! BL-CON-05 (#284): typed deductions on a certificate (`CON.ACL.14`). The QS
//! records each line with its reference, the project manager accepts or
//! contests it (never a line they recorded), accepting one above AED 25,000
//! needs the MD, and the IPA is certified only when every line is decided.
//! Contested lines are listed with the project's final account items.

mod support;

use construction::permissions::{PROJECT_MANAGER_ROLE, QS_ROLE};
use construction::{DeductionStatus, IpaDeduction, IpaStatus, ProjectStatus, deduction};
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, approve, reject};

use support::{Booted, boot, date};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

/// IPA 6 of a running project is `submitted`; returns its id and the project.
async fn six(t: &Booted, manager: &User) -> (construction::Project, i64) {
    let project = t.project(ProjectStatus::InProgress, Some(manager)).await;
    t.ipa(
        &project,
        5,
        IpaStatus::Certified,
        aed(520_000),
        Some(aed(520_000)),
    )
    .await;
    let six = t
        .ipa(&project, 6, IpaStatus::Submitted, aed(200_000), None)
        .await;
    (project, six.id)
}

/// `by` records a line on `ipa` and returns it.
async fn record(
    t: &Booted,
    by: &User,
    ipa: i64,
    kind: &str,
    reference: &str,
    amount: i64,
) -> IpaDeduction {
    let amount_text = amount.to_string();
    t.post(
        by,
        &format!("/construction/ipas/{ipa}/deductions"),
        &[
            ("kind", kind),
            ("reference", reference),
            ("amount", amount_text.as_str()),
            ("description", "Scaffolding and cleaning"),
        ],
    )
    .await
    .assert_ok();
    deduction::lines_of(t.db(), ipa)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.reference == reference)
        .expect("the line")
}

async fn reload(t: &Booted, line: &IpaDeduction) -> IpaDeduction {
    IpaDeduction::find_or_404(t.db(), line.id).await.unwrap()
}

async fn certificate(t: &Booted, by: &User, ipa: i64) -> renox::testing::TestResponse {
    let amount = aed(180_000).to_string();
    t.post(
        by,
        &format!("/construction/ipas/{ipa}/certificate"),
        &[
            ("certified_amount", amount.as_str()),
            ("certificate", "CERT-6.pdf"),
        ],
    )
    .await
}

async fn waiting_request(line: &IpaDeduction, t: &Booted) -> Option<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &line.subject())
        .await
        .unwrap()
        .into_iter()
        .next()
}

/// Done when 1 to 4: the QS records a certificate of AED 180,000 with the
/// lines BC-12 (5,000), LD-1 (30,000) and CC-3 (12,000); `certify` is refused
/// while any is undecided; the project manager accepts BC-12 directly, LD-1
/// goes to the MD, and CC-3 is contested; after the MD approves, `certify`
/// succeeds and CC-3 is listed with the final account items.
#[renox::test]
async fn the_lines_are_decided_the_md_approves_the_big_one_and_the_ipa_is_certified() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let md = t.md("Bader").await;
    let (project, six) = six(&t, &pm).await;

    t.on(date(2027, 3, 10), async {
        let bc = record(&t, &qs, six, "back_charge", "BC-12", aed(5_000)).await;
        let ld = record(&t, &qs, six, "liquidated_damages", "LD-1", aed(30_000)).await;
        let cc = record(&t, &qs, six, "contra_charge", "CC-3", aed(12_000)).await;
        assert_eq!(bc.status, DeductionStatus::Undecided);
        assert_eq!(bc.recorded_by, qs.id);

        // Undecided lines stop the certificate.
        let message = {
            let refused = certificate(&t, &qs, six).await;
            refused.assert_status(422);
            refused.text()
        };
        assert!(message.contains("3 deduction"), "{message}");

        // BC-12 is accepted directly.
        t.post(
            &pm,
            &format!("/construction/deductions/{}/accept", bc.id),
            &[],
        )
        .await
        .assert_ok();
        let bc = reload(&t, &bc).await;
        assert_eq!(bc.status, DeductionStatus::Accepted);
        assert_eq!(bc.decided_by, Some(pm.id));
        assert!(bc.decided_at.is_some());

        // LD-1 is above AED 25,000: the MD is asked, the line waits.
        t.post(
            &pm,
            &format!("/construction/deductions/{}/accept", ld.id),
            &[],
        )
        .await
        .assert_ok();
        assert_eq!(reload(&t, &ld).await.status, DeductionStatus::Undecided);
        let request = waiting_request(&ld, &t).await.expect("an approval request");
        assert_eq!(request.requester_id, pm.id);

        // CC-3 is contested.
        t.post(
            &pm,
            &format!("/construction/deductions/{}/contest", cc.id),
            &[],
        )
        .await
        .assert_ok();
        assert_eq!(reload(&t, &cc).await.status, DeductionStatus::Contested);

        // LD-1 still waits for the MD.
        certificate(&t, &qs, six).await.assert_status(422);
        assert_eq!(t.reload_ipa(six).await.status, IpaStatus::Submitted);

        // The requester cannot approve; the MD does.
        let refused = approve(t.state(), request.id, &pm, "").await;
        assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
        approve(t.state(), request.id, &md, "agreed").await.unwrap();
        let ld = reload(&t, &ld).await;
        assert_eq!(ld.status, DeductionStatus::Accepted);
        assert_eq!(ld.decided_by, Some(pm.id));

        // Now the IPA is certified; the deductions are not netted in.
        certificate(&t, &qs, six).await.assert_ok();
    })
    .await;

    let certified = t.reload_ipa(six).await;
    assert_eq!(certified.status, IpaStatus::Certified);
    assert_eq!(certified.certified_amount, Some(aed(180_000)));
    assert_eq!(certified.claim_gap, Some(aed(20_000)));

    // CC-3 is listed with the final account items.
    let items = deduction::final_account_items(t.db(), project.id)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reference, "CC-3");
    assert_eq!(items[0].amount, aed(12_000));
    let closing = t
        .get(
            &pm,
            &format!("/construction/projects/{}/closing", project.id),
        )
        .await;
    closing.assert_ok();
    let view: renox::serde_json::Value = renox::serde_json::from_str(&closing.text()).unwrap();
    assert_eq!(view["contested_deductions"][0]["reference"], "CC-3");

    // The IPA's screen lists the three lines.
    let shown = t.get(&qs, &format!("/construction/ipas/{six}")).await;
    let shown: renox::serde_json::Value = renox::serde_json::from_str(&shown.text()).unwrap();
    assert_eq!(shown["deductions"].as_array().map(Vec::len), Some(3));
    assert_eq!(shown["deductions"][1]["kind"], "liquidated_damages");
}

/// A line of exactly AED 25,000 is accepted without the MD; a rejection of
/// the MD leaves the line undecided and the project manager contests it.
#[renox::test]
async fn the_limit_is_per_line_and_a_rejection_leaves_the_line_undecided() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let md = t.md("Bader").await;
    let (_project, six) = six(&t, &pm).await;

    t.on(date(2027, 3, 10), async {
        let limit = record(&t, &qs, six, "back_charge", "BC-25", aed(25_000)).await;
        let above = record(&t, &qs, six, "back_charge", "BC-26", aed(25_000) + 1).await;
        t.post(
            &pm,
            &format!("/construction/deductions/{}/accept", limit.id),
            &[],
        )
        .await
        .assert_ok();
        assert_eq!(reload(&t, &limit).await.status, DeductionStatus::Accepted);
        assert!(waiting_request(&limit, &t).await.is_none());

        t.post(
            &pm,
            &format!("/construction/deductions/{}/accept", above.id),
            &[],
        )
        .await
        .assert_ok();
        let request = waiting_request(&above, &t).await.expect("a request");
        reject(t.state(), request.id, &md, "too high")
            .await
            .unwrap();
        assert_eq!(reload(&t, &above).await.status, DeductionStatus::Undecided);

        t.post(
            &pm,
            &format!("/construction/deductions/{}/contest", above.id),
            &[],
        )
        .await
        .assert_ok();
        assert_eq!(reload(&t, &above).await.status, DeductionStatus::Contested);
    })
    .await;
}

/// Done when 5: the user who recorded a line cannot accept or contest it,
/// and the QS alone holds no right to decide.
#[renox::test]
async fn the_qs_cannot_accept_the_lines_they_recorded() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    // A person who is both the QS and a project manager.
    let both = t.user_with("Basma", QS_ROLE).await;
    both.assign_role(t.db(), PROJECT_MANAGER_ROLE)
        .await
        .unwrap();
    let (_project, six) = six(&t, &pm).await;

    t.on(date(2027, 3, 10), async {
        let own = record(&t, &both, six, "back_charge", "BC-1", aed(1_000)).await;
        let path = |action: &str, line: &IpaDeduction| {
            format!("/construction/deductions/{}/{action}", line.id)
        };
        // The right to decide is not enough for a line one recorded.
        let message = {
            let refused = t.post(&both, &path("accept", &own), &[]).await;
            refused.assert_status(422);
            refused.text()
        };
        assert!(message.contains("recorded"), "{message}");
        t.post(&both, &path("contest", &own), &[])
            .await
            .assert_status(422);
        // The plain QS holds no right to decide at all.
        let theirs = record(&t, &qs, six, "back_charge", "BC-2", aed(1_000)).await;
        t.post(&qs, &path("accept", &theirs), &[])
            .await
            .assert_status(403);
        assert_eq!(reload(&t, &own).await.status, DeductionStatus::Undecided);
        assert_eq!(reload(&t, &theirs).await.status, DeductionStatus::Undecided);
        // Another project manager decides the line.
        t.post(&pm, &path("accept", &own), &[]).await.assert_ok();
        assert_eq!(reload(&t, &own).await.status, DeductionStatus::Accepted);
    })
    .await;
}

/// A line needs a reference, an amount and a known type, the type `other`
/// needs a description, only the QS records, and lines close at `certify`.
#[renox::test]
async fn a_line_is_checked_and_lines_close_when_the_ipa_is_certified() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let (_project, six) = six(&t, &pm).await;
    let path = format!("/construction/ipas/{six}/deductions");

    t.on(date(2027, 3, 10), async {
        let form = |kind: &'static str, reference: &'static str, amount: &'static str| {
            [
                ("kind", kind),
                ("reference", reference),
                ("amount", amount),
                ("description", ""),
            ]
        };
        t.post(&qs, &path, &form("bribe", "X-1", "100"))
            .await
            .assert_status(422);
        t.post(&qs, &path, &form("back_charge", " ", "100"))
            .await
            .assert_status(422);
        t.post(&qs, &path, &form("back_charge", "X-1", "0"))
            .await
            .assert_status(422);
        t.post(&qs, &path, &form("other", "X-1", "100"))
            .await
            .assert_status(422);
        t.post(&pm, &path, &form("back_charge", "X-1", "100"))
            .await
            .assert_status(403);
        assert!(deduction::lines_of(t.db(), six).await.unwrap().is_empty());

        // No deductions: `certify` needs only the amount and the certificate.
        certificate(&t, &qs, six).await.assert_ok();
        let closed = t.post(&qs, &path, &form("back_charge", "X-2", "100")).await;
        closed.assert_status(422);
        assert!(closed.text().contains("no longer"), "{}", closed.text());
    })
    .await;
    assert_eq!(t.reload_ipa(six).await.status, IpaStatus::Certified);
}
