//! BL-CON-12 (#291): the kind of a variation. An omission cannot omit more
//! than the work not yet done and reduces the contract value, the budget and
//! the next IPA when approved; a time variation states the days asked and the
//! delay event and, approved with days granted, moves the completion date; an
//! addition approved at less than priced shows the gap in the variation
//! report.

mod support;

use construction::boq::{self, BoqLine};
use construction::variation::{self, Variation};
use construction::{IpaStatus, Project, ProjectStatus, VariationStatus};
use renox::prelude::*;

use support::{Booted, boot, date, users_of};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

fn path(variation: &Variation, step: &str) -> String {
    format!("/construction/variations/{}/{step}", variation.id)
}

/// A project in progress with a contract value of AED 2,000,000 and its
/// contract completion date on 2027-10-31.
async fn project(t: &Booted, manager: &User) -> Project {
    let project = t.project(ProjectStatus::InProgress, Some(manager)).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            t.db(),
            &[
                ("contract_value", &aed(2_000_000)),
                ("completion_due_on", &Some(date(2027, 10, 31))),
            ],
        )
        .await
        .unwrap();
    t.reload(&project).await
}

/// BOQ line 4.2 of 1,000 m² at AED 120, with 700 m² done on the draft IPA.
async fn line_with_progress(t: &Booted, project: &Project) -> BoqLine {
    let mut line = BoqLine::new(project.id, "4.2", 1_000, aed(120));
    line.unit = "m²".into();
    line.save(t.db()).await.unwrap();
    let ipa = t.ipa(project, 1, IpaStatus::Draft, 0, None).await;
    boq::record_progress(t.db(), ipa.id, line.id, 700)
        .await
        .unwrap();
    line
}

/// The QS records an omission of `quantity` from `line`.
async fn omission(
    t: &Booted,
    qs: &User,
    project: &Project,
    line: &BoqLine,
    quantity: i64,
) -> Variation {
    t.post(
        qs,
        &format!("/construction/projects/{}/variations", project.id),
        &[
            ("kind", "omission"),
            ("description", "Omit the screed on level 4"),
            ("boq_line_id", &line.id.to_string()),
            ("omitted_quantity", &quantity.to_string()),
        ],
    )
    .await
    .assert_ok();
    variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .expect("the variation")
}

/// Done when 1: BOQ line 4.2 of 1,000 m² with 700 m² done, an omission of 400
/// m² is refused at `submit`; 300 m² at AED 120 is submitted.
#[renox::test]
async fn an_omission_cannot_omit_more_than_is_not_yet_done() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let line = line_with_progress(&t, &project).await;

    let too_much = omission(&t, &qs, &project, &line, 400).await;
    assert_eq!(too_much.kind, construction::VariationKind::Omission);
    assert_eq!(
        too_much.priced_amount,
        -aed(48_000),
        "the omitted quantity at the rate of the line, negative"
    );
    assert_eq!(boq::not_yet_done(t.db(), &line).await.unwrap(), 300);
    let refused = t.post(&pm, &path(&too_much, "submit"), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("400"), "{}", refused.text());
    assert!(refused.text().contains("300"), "{}", refused.text());
    assert_eq!(
        t.reload_variation(too_much.id).await.status,
        VariationStatus::Draft
    );

    // The QS prices 300 m² instead; the project manager submits it.
    t.post(
        &qs,
        &path(&too_much, "price"),
        &[
            ("kind", "omission"),
            ("description", "Omit the screed on level 4"),
            ("boq_line_id", &line.id.to_string()),
            ("omitted_quantity", "300"),
        ],
    )
    .await
    .assert_ok();
    let fits = t.reload_variation(too_much.id).await;
    assert_eq!(fits.priced_amount, -aed(36_000));
    t.post(&pm, &path(&fits, "submit"), &[]).await.assert_ok();
    assert_eq!(
        t.reload_variation(fits.id).await.status,
        VariationStatus::Submitted
    );

    // An omission names its line and quantity.
    t.post(
        &qs,
        &format!("/construction/projects/{}/variations", project.id),
        &[
            ("kind", "omission"),
            ("description", "Omit something"),
            ("priced_amount", "100000"),
        ],
    )
    .await
    .assert_ok();
    let unnamed = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(unnamed.priced_amount, -aed(1_000));
    t.post(&pm, &path(&unnamed, "submit"), &[])
        .await
        .assert_status(422);

    // The latest recorded progress counts: with no draft IPA, the latest
    // submitted one.
    let later = t.reload_variation(fits.id).await;
    assert_eq!(later.omitted_quantity, Some(300));
}

/// Done when 2: approving the omission at AED −36,000 reduces the contract
/// value, the budget and the next IPA by AED 36,000.
#[renox::test]
async fn an_approved_omission_reduces_the_contract_value_the_budget_and_the_next_ipa() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let line = line_with_progress(&t, &project).await;
    let made = omission(&t, &qs, &project, &line, 300).await;
    t.post(&pm, &path(&made, "submit"), &[]).await.assert_ok();
    let before = t.reload(&project).await;

    t.post(
        &qs,
        &path(&made, "approve"),
        &[
            ("client_reference", "CL-31"),
            ("approved_amount", "-3600000"),
        ],
    )
    .await
    .assert_ok();
    let approved = t.reload_variation(made.id).await;
    assert_eq!(approved.status, VariationStatus::Approved);
    assert_eq!(approved.approved_amount, Some(-aed(36_000)));
    assert_eq!(approved.excess_quantity, 0);
    let after = t.reload(&project).await;
    assert_eq!(after.contract_value, before.contract_value - aed(36_000));
    assert_eq!(after.budget, before.budget - aed(36_000));
    assert_eq!(
        variation::next_ipa_amount(t.db(), project.id)
            .await
            .unwrap(),
        -aed(36_000)
    );
    assert!(t.notices.notices_titled("overtaken").is_empty());
}

/// An omission that progress has overtaken by the time the client approves it
/// is still approved; the QS is warned to record the excess as a claim.
#[renox::test]
async fn an_omission_overtaken_by_progress_is_approved_and_the_qs_is_warned() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let line = line_with_progress(&t, &project).await;
    let made = omission(&t, &qs, &project, &line, 300).await;
    t.post(&pm, &path(&made, "submit"), &[]).await.assert_ok();

    // Progress moves on to 800 m² while the client thinks.
    let draft = boq::progress_ipa(t.db(), project.id)
        .await
        .unwrap()
        .unwrap();
    boq::record_progress(t.db(), draft.id, line.id, 800)
        .await
        .unwrap();
    assert_eq!(boq::not_yet_done(t.db(), &line).await.unwrap(), 200);

    t.post(
        &qs,
        &path(&made, "approve"),
        &[
            ("client_reference", "CL-31"),
            ("approved_amount", "3600000"),
        ],
    )
    .await
    .assert_ok();
    let approved = t.reload_variation(made.id).await;
    assert_eq!(approved.status, VariationStatus::Approved);
    assert_eq!(
        approved.approved_amount,
        Some(-aed(36_000)),
        "stored negative"
    );
    assert_eq!(approved.excess_quantity, 100);
    let told = t.notices.notices_titled("overtaken");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), [qs.id]);
    assert!(told[0].data["body"].as_str().unwrap().contains("100 m²"));
    assert!(told[0].data["body"].as_str().unwrap().contains("claim"));
}

/// The latest recorded progress is the draft IPA's if there is one, else the
/// latest submitted or certified IPA's.
#[renox::test]
async fn the_quantity_not_yet_done_comes_from_the_latest_recorded_progress() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = project(&t, &pm).await;
    let mut line = BoqLine::new(project.id, "4.2", 1_000, aed(120));
    line.save(t.db()).await.unwrap();
    assert_eq!(boq::not_yet_done(t.db(), &line).await.unwrap(), 1_000);

    let certified = t
        .ipa(&project, 1, IpaStatus::Certified, aed(10), Some(aed(10)))
        .await;
    boq::record_progress(t.db(), certified.id, line.id, 400)
        .await
        .unwrap();
    assert_eq!(boq::not_yet_done(t.db(), &line).await.unwrap(), 600);

    let submitted = t
        .ipa(&project, 2, IpaStatus::Submitted, aed(10), None)
        .await;
    boq::record_progress(t.db(), submitted.id, line.id, 550)
        .await
        .unwrap();
    assert_eq!(boq::not_yet_done(t.db(), &line).await.unwrap(), 450);

    // The draft IPA wins, even with less recorded.
    let draft = t.ipa(&project, 3, IpaStatus::Draft, 0, None).await;
    boq::record_progress(t.db(), draft.id, line.id, 700)
        .await
        .unwrap();
    assert_eq!(boq::not_yet_done(t.db(), &line).await.unwrap(), 300);
    let progress = boq::progress_ipa(t.db(), project.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(progress.id, draft.id);
}

/// Done when 3: a time variation asking 30 days for delay event "late
/// drawings, 2027-04-01", approved for 20 days, moves the completion date from
/// 2027-10-31 to 2027-11-20.
#[renox::test]
async fn an_approved_extension_of_time_moves_the_completion_date() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    let create = format!("/construction/projects/{}/variations", project.id);

    // The days asked and the delay event are needed to submit.
    t.post(
        &qs,
        &create,
        &[("kind", "time"), ("description", "Extension of time")],
    )
    .await
    .assert_ok();
    let bare = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let refused = t.post(&pm, &path(&bare, "submit"), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("days"), "{}", refused.text());
    t.post(
        &qs,
        &path(&bare, "price"),
        &[
            ("kind", "time"),
            ("description", "Extension of time"),
            ("days_asked", "30"),
        ],
    )
    .await
    .assert_ok();
    let refused = t.post(&pm, &path(&bare, "submit"), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("event"), "{}", refused.text());

    t.post(
        &qs,
        &path(&bare, "price"),
        &[
            ("kind", "time"),
            ("description", "Extension of time"),
            ("days_asked", "30"),
            ("event", "late drawings"),
            ("event_date", "2027-04-01"),
        ],
    )
    .await
    .assert_ok();
    t.post(&pm, &path(&bare, "submit"), &[]).await.assert_ok();
    let sent = t.reload_variation(bare.id).await;
    assert_eq!(sent.status, VariationStatus::Submitted);
    assert_eq!(sent.days_asked, Some(30));
    assert_eq!(sent.event.as_deref(), Some("late drawings"));
    assert_eq!(sent.event_date, Some(date(2027, 4, 1)));
    assert_eq!(
        t.reload(&project).await.completion_due_on,
        Some(date(2027, 10, 31))
    );

    // The client's reference and the days granted are needed.
    let approve = path(&sent, "approve");
    t.post(&qs, &approve, &[("client_reference", "CL-40")])
        .await
        .assert_status(422);
    t.post(&qs, &approve, &[("days_granted", "20")])
        .await
        .assert_status(422);
    t.post(
        &qs,
        &approve,
        &[("client_reference", "CL-40"), ("days_granted", "20")],
    )
    .await
    .assert_ok();
    let granted = t.reload_variation(bare.id).await;
    assert_eq!(granted.status, VariationStatus::Approved);
    assert_eq!(granted.days_granted, Some(20));
    assert_eq!(
        granted.approved_amount, None,
        "no amount for a time variation"
    );
    let moved = t.reload(&project).await;
    assert_eq!(moved.completion_due_on, Some(date(2027, 11, 20)));
    assert_eq!(moved.contract_value, aed(2_000_000), "no money moves");
    assert_eq!(
        variation::next_ipa_amount(t.db(), project.id)
            .await
            .unwrap(),
        0
    );

    // 0 days granted is a decision too: the date does not move.
    t.post(
        &qs,
        &create,
        &[
            ("kind", "time"),
            ("description", "Second extension"),
            ("days_asked", "10"),
            ("event", "late approvals"),
            ("event_date", "2027-05-01"),
        ],
    )
    .await
    .assert_ok();
    let second = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    t.post(&pm, &path(&second, "submit"), &[]).await.assert_ok();
    t.post(
        &qs,
        &path(&second, "approve"),
        &[("client_reference", "CL-41"), ("days_granted", "0")],
    )
    .await
    .assert_ok();
    assert_eq!(t.reload_variation(second.id).await.days_granted, Some(0));
    assert_eq!(
        t.reload(&project).await.completion_due_on,
        Some(date(2027, 11, 20))
    );
}

/// Done when 4: an addition priced AED 50,000 approved at AED 42,000 shows a
/// gap of AED 8,000 in the variation report.
#[renox::test]
async fn the_variation_report_shows_what_the_client_approved_less_than_priced() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project(&t, &pm).await;
    t.post(
        &qs,
        &format!("/construction/projects/{}/variations", project.id),
        &[
            ("kind", "addition"),
            ("description", "Extra partition on level 3"),
            ("priced_amount", "5000000"),
        ],
    )
    .await
    .assert_ok();
    let made = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    t.post(&pm, &path(&made, "submit"), &[]).await.assert_ok();
    let before = t.reload(&project).await;
    t.post(
        &qs,
        &path(&made, "approve"),
        &[
            ("client_reference", "CL-50"),
            ("approved_amount", "4200000"),
        ],
    )
    .await
    .assert_ok();
    let after = t.reload(&project).await;
    assert_eq!(after.contract_value, before.contract_value + aed(42_000));
    assert_eq!(after.budget, before.budget + aed(42_000));

    let report = t
        .get(
            &pm,
            &format!("/construction/projects/{}/variation-report", project.id),
        )
        .await;
    report.assert_ok();
    let lines: renox::serde_json::Value = renox::serde_json::from_str(&report.text()).unwrap();
    assert_eq!(lines[0]["code"], "VO-01");
    assert_eq!(lines[0]["priced_amount"], aed(50_000));
    assert_eq!(lines[0]["approved_amount"], aed(42_000));
    assert_eq!(lines[0]["gap"], aed(8_000));
    let line = construction::variation::report_of(t.db(), project.id)
        .await
        .unwrap();
    assert_eq!(line[0].gap, Some(aed(8_000)));
    // Anyone without a part in the project is refused.
    let outsider = t.accountant("Olga").await;
    t.get(
        &outsider,
        &format!("/construction/projects/{}/variation-report", project.id),
    )
    .await
    .assert_status(403);
}
