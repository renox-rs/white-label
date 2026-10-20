//! BL-CON-04 (#283): an IPA still waiting for its certificate at the next
//! cut-off (`CON.RUL.03`, `supersede`). The monthly draft is still made,
//! marked *previous pending*; the new IPA is submitted only as a replacement
//! of the pending one, which becomes `superseded` while the client is told; a
//! late certificate for the superseded IPA is recorded on the replacing one.

mod support;

use construction::{Ipa, IpaStatus, Project, ProjectStatus, cutoff};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{Booted, boot, date};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

/// The message of a refusal (a 422).
fn refusal(response: &renox::testing::TestResponse) -> String {
    response.assert_status(422);
    response.text()
}

/// Sets what a draft claims: the progress of the period.
async fn claim(t: &Booted, id: i64, fils: i64) {
    Ipa::query()
        .where_(Ipa::ID.eq(id))
        .update(t.db(), &[("claimed_amount", &fils)])
        .await
        .unwrap();
}

/// A project with IPA 4 invoiced (AED 400,000 certified) and IPA 5 submitted
/// without a certificate (it claims AED 520,000).
async fn waiting(t: &Booted, manager: &User) -> (Project, i64, i64) {
    let project = t.project(ProjectStatus::InProgress, Some(manager)).await;
    let four = t
        .ipa(
            &project,
            4,
            IpaStatus::Invoiced,
            aed(400_000),
            Some(aed(400_000)),
        )
        .await;
    let five = t
        .ipa(&project, 5, IpaStatus::Submitted, aed(520_000), None)
        .await;
    (project, four.id, five.id)
}

/// Done when 1 and 5: on 2027-02-25 the cut-off makes draft IPA 6 marked
/// *previous pending*, tells the QS and the project manager, and its previous
/// certified is AED 400,000; a second run the same day makes nothing.
#[renox::test]
async fn the_cutoff_marks_the_draft_previous_pending() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let _qs = t.qs("Qasim").await;
    let (project, _four, _five) = waiting(&t, &pm).await;

    let run = t
        .on(date(2027, 2, 25), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    assert_eq!(run.drafts.len(), 1);

    let draft = t.ipa_numbered(&project, 6).await;
    assert_eq!(draft.status, IpaStatus::Draft);
    assert!(draft.previous_pending);
    assert_eq!(draft.previous_certified, aed(400_000));
    assert_eq!(draft.cutoff_on, Some(date(2027, 2, 25)));
    assert_eq!(t.tasks.with_key("construction.ipa_cutoff_qs:").len(), 1);
    let for_manager = t.tasks.with_key("construction.ipa_cutoff_pm:");
    assert_eq!(for_manager.len(), 1);
    assert!(for_manager[0].ticket.description.contains("IPA 5"));

    let again = t
        .on(date(2027, 2, 25), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    assert!(again.drafts.is_empty());
    assert_eq!(t.tasks.with_key("construction.ipa_cutoff").len(), 2);
    assert_eq!(ipa_count(&t, &project).await, 3);
}

async fn ipa_count(t: &Booted, project: &Project) -> usize {
    construction::ipa::count_of(t.db(), project.id)
        .await
        .unwrap()
}

/// The cut-off waits for its day, skips a project with a draft or one that is
/// not running, and without a pending IPA only the QS is told.
#[renox::test]
async fn the_cutoff_runs_on_its_day_for_running_projects_without_a_draft() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let running = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let held = t.project(ProjectStatus::OnHold, Some(&pm)).await;
    let drafted = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    t.ipa(&drafted, 1, IpaStatus::Draft, 0, None).await;
    t.ipa(
        &running,
        1,
        IpaStatus::Certified,
        aed(100_000),
        Some(aed(90_000)),
    )
    .await;

    let early = t
        .on(date(2027, 2, 24), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    assert!(early.drafts.is_empty());

    let run = t
        .on(date(2027, 2, 25), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    assert_eq!(run.drafts.len(), 1);
    let draft = t.ipa_numbered(&running, 2).await;
    assert!(!draft.previous_pending);
    assert_eq!(draft.previous_certified, aed(90_000));
    assert_eq!(t.tasks.with_key("construction.ipa_cutoff_qs:").len(), 1);
    assert!(t.tasks.with_key("construction.ipa_cutoff_pm:").is_empty());
    assert_eq!(ipa_count(&t, &held).await, 0);
    assert_eq!(ipa_count(&t, &drafted).await, 1);
}

/// Done when 2: submitting IPA 6 without "replaces IPA 5" is refused; with it
/// IPA 5 is `superseded`, the client is told and IPA 6 is `submitted`.
#[renox::test]
async fn a_pending_ipa_is_replaced_explicitly_and_the_client_is_told() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let (project, _four, five) = waiting(&t, &pm).await;
    t.on(date(2027, 2, 25), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    let draft = t.ipa_numbered(&project, 6).await;
    let path = format!("/construction/ipas/{}/submit", draft.id);

    // No progress yet.
    let message = t
        .on(date(2027, 2, 26), async {
            refusal(
                &t.post(&pm, &path, &[("replaces", five.to_string().as_str())])
                    .await,
            )
        })
        .await;
    assert!(message.contains("progress"), "{message}");

    claim(&t, draft.id, aed(560_000)).await;
    let message = t
        .on(date(2027, 2, 26), async {
            refusal(&t.post(&pm, &path, &[]).await)
        })
        .await;
    assert!(message.contains("IPA 5"), "{message}");
    assert_eq!(t.reload_ipa(draft.id).await.status, IpaStatus::Draft);
    assert_eq!(t.reload_ipa(five).await.status, IpaStatus::Submitted);

    t.on(date(2027, 2, 26), async {
        t.post(&pm, &path, &[("replaces", five.to_string().as_str())])
            .await
            .assert_ok();
    })
    .await;
    let replacing = t.reload_ipa(draft.id).await;
    assert_eq!(replacing.status, IpaStatus::Submitted);
    assert_eq!(replacing.replaces_id, Some(five));
    assert_eq!(replacing.submitted_on, Some(date(2027, 2, 26)));
    assert_eq!(t.reload_ipa(five).await.status, IpaStatus::Superseded);
    assert_eq!(t.notices.of_type("construction.ipa_superseded").len(), 1);
}

/// Only the most recent pending IPA can be replaced and nothing else is
/// submitted while one waits.
#[renox::test]
async fn two_ipas_are_never_submitted_at_once() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let (project, four, five) = waiting(&t, &pm).await;
    let next = t
        .ipa(&project, 6, IpaStatus::Draft, aed(560_000), None)
        .await;
    let path = format!("/construction/ipas/{}/submit", next.id);

    let message = t
        .on(date(2027, 2, 26), async {
            refusal(
                &t.post(&pm, &path, &[("replaces", four.to_string().as_str())])
                    .await,
            )
        })
        .await;
    assert!(message.contains("most recent"), "{message}");
    assert_eq!(t.reload_ipa(next.id).await.status, IpaStatus::Draft);
    assert_eq!(t.reload_ipa(five).await.status, IpaStatus::Submitted);
}

/// Done when 3: the client's certificate for IPA 5 (AED 520,000 cumulative)
/// arriving on 2027-03-05 is recorded as IPA 6's certificate and IPA 6's
/// claim gap is shown.
#[renox::test]
async fn a_late_certificate_is_recorded_on_the_replacing_ipa() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let (project, _four, five) = waiting(&t, &pm).await;
    t.on(date(2027, 2, 25), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    let six = t.ipa_numbered(&project, 6).await;
    claim(&t, six.id, aed(560_000)).await;
    t.on(date(2027, 2, 26), async {
        t.post(
            &pm,
            &format!("/construction/ipas/{}/submit", six.id),
            &[("replaces", five.to_string().as_str())],
        )
        .await
        .assert_ok();
    })
    .await;

    t.on(date(2027, 3, 5), async {
        t.post(
            &qs,
            &format!("/construction/ipas/{five}/certificate"),
            &[
                ("certified_amount", aed(520_000).to_string().as_str()),
                ("certificate", "CERT-5.pdf"),
            ],
        )
        .await
        .assert_ok();
    })
    .await;

    assert_eq!(t.reload_ipa(five).await.status, IpaStatus::Superseded);
    assert_eq!(t.reload_ipa(five).await.certified_amount, None);
    let certified = t.reload_ipa(six.id).await;
    assert_eq!(certified.status, IpaStatus::Certified);
    assert_eq!(certified.certified_amount, Some(aed(520_000)));
    assert_eq!(certified.certificate_ref.as_deref(), Some("CERT-5.pdf"));
    assert_eq!(certified.certified_on, Some(date(2027, 3, 5)));
    assert_eq!(certified.claim_gap, Some(aed(40_000)));

    let shown = t.get(&qs, &format!("/construction/ipas/{}", six.id)).await;
    shown.assert_ok();
    let view: renox::serde_json::Value = renox::serde_json::from_str(&shown.text()).unwrap();
    assert_eq!(view["claim_gap"], aed(40_000));
    assert_eq!(view["status"], "certified");
    assert_eq!(view["replaces"], five);
}

/// When the replacing IPA is certified already, the late certificate is
/// attached for reference only: no amount changes and the QS is told.
#[renox::test]
async fn a_late_certificate_after_the_replacing_one_is_certified_changes_no_amount() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let (project, _four, five) = waiting(&t, &pm).await;
    let six = t
        .ipa(&project, 6, IpaStatus::Submitted, aed(560_000), None)
        .await;
    Ipa::query()
        .where_(Ipa::ID.eq(six.id))
        .update(t.db(), &[("replaces_id", &Some(five))])
        .await
        .unwrap();
    // IPA 5 was replaced: it is superseded.
    let mut tx = t.db().begin().await.unwrap();
    let old = t.reload_ipa(five).await;
    let outcome = old
        .transition(construction::ipa::SUPERSEDE)
        .by_system()
        .run_in(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    outcome.emit(t.state()).await.unwrap();

    let certify = |amount: i64, reference: &'static str, id: i64| {
        let t = &t;
        let qs = &qs;
        async move {
            t.post(
                qs,
                &format!("/construction/ipas/{id}/certificate"),
                &[
                    ("certified_amount", amount.to_string().as_str()),
                    ("certificate", reference),
                ],
            )
            .await
        }
    };
    t.on(date(2027, 3, 1), async {
        certify(aed(540_000), "CERT-6.pdf", six.id)
            .await
            .assert_ok();
        certify(aed(520_000), "CERT-5-late.pdf", five)
            .await
            .assert_ok();
    })
    .await;

    let kept = t.reload_ipa(six.id).await;
    assert_eq!(kept.certified_amount, Some(aed(540_000)));
    assert_eq!(kept.certificate_ref.as_deref(), Some("CERT-6.pdf"));
    assert_eq!(
        kept.late_certificate_ref.as_deref(),
        Some("CERT-5-late.pdf")
    );
    assert_eq!(kept.claim_gap, Some(aed(20_000)));
    assert_eq!(
        t.tasks.with_key("construction.ipa_late_certificate:").len(),
        1
    );
    // A draft IPA cannot take a certificate.
    let draft = t.ipa(&project, 7, IpaStatus::Draft, 0, None).await;
    t.on(date(2027, 3, 1), async {
        certify(1, "X.pdf", draft.id).await.assert_status(422);
    })
    .await;
}

/// Done when 4: IPA 5 certified on 2027-03-01 before draft IPA 6 is
/// submitted clears the *previous pending* flag; IPA 6's previous certified
/// becomes IPA 5's amount and it submits normally.
#[renox::test]
async fn certifying_the_pending_ipa_clears_the_flag_on_the_draft() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let (project, _four, five) = waiting(&t, &pm).await;
    t.on(date(2027, 2, 25), async { cutoff::run(t.state()).await })
        .await
        .unwrap();
    let six = t.ipa_numbered(&project, 6).await;
    assert!(six.previous_pending);

    t.on(date(2027, 3, 1), async {
        t.post(
            &qs,
            &format!("/construction/ipas/{five}/certificate"),
            &[
                ("certified_amount", aed(520_000).to_string().as_str()),
                ("certificate", "CERT-5.pdf"),
            ],
        )
        .await
        .assert_ok();
    })
    .await;
    let five_now = t.reload_ipa(five).await;
    assert_eq!(five_now.status, IpaStatus::Certified);
    assert_eq!(five_now.claim_gap, Some(0));
    let refreshed = t.reload_ipa(six.id).await;
    assert!(!refreshed.previous_pending);
    assert_eq!(refreshed.previous_certified, aed(520_000));

    claim(&t, six.id, aed(600_000)).await;
    t.on(date(2027, 3, 2), async {
        t.post(&pm, &format!("/construction/ipas/{}/submit", six.id), &[])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(t.reload_ipa(six.id).await.status, IpaStatus::Submitted);
    assert_eq!(t.reload_ipa(five).await.status, IpaStatus::Certified);
}

/// BL-CON-03 done when 2: the IPA of a blocked client's running project is
/// submitted as usual.
#[renox::test]
async fn a_blocked_clients_project_still_submits_its_ipa() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let draft = t
        .ipa(&project, 1, IpaStatus::Draft, aed(100_000), None)
        .await;
    t.blocked.block(support::CLIENT_PARTY);

    t.on(date(2027, 3, 2), async {
        t.post(&pm, &format!("/construction/ipas/{}/submit", draft.id), &[])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(t.reload_ipa(draft.id).await.status, IpaStatus::Submitted);
    assert_eq!(t.reload(&project).await.status, ProjectStatus::InProgress);
}

/// Without the permission the actions are refused.
#[renox::test]
async fn submitting_and_certifying_need_their_permissions() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = t.project(ProjectStatus::InProgress, Some(&pm)).await;
    let draft = t
        .ipa(&project, 1, IpaStatus::Draft, aed(100_000), None)
        .await;
    let submitted = t
        .ipa(&project, 2, IpaStatus::Submitted, aed(100_000), None)
        .await;
    t.on(date(2027, 3, 2), async {
        t.post(&qs, &format!("/construction/ipas/{}/submit", draft.id), &[])
            .await
            .assert_status(403);
        t.post(
            &pm,
            &format!("/construction/ipas/{}/certificate", submitted.id),
            &[("certified_amount", "1"), ("certificate", "C.pdf")],
        )
        .await
        .assert_status(403);
    })
    .await;
}
