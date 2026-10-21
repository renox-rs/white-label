//! The HTML pages of the construction screens (the projects list, an IPA,
//! the closing screen, the variation report): a browser, which names
//! `text/html` in `Accept`, gets a page with the key facts and the links to
//! the related records; a request without it gets the JSON it always got.

mod support;

use construction::variation;
use construction::{
    DeductionKind, DeductionStatus, Guarantee, GuaranteeKind, GuaranteeStatus, Ipa, IpaKind,
    IpaStatus, Project, ProjectStatus, Retention, RetentionPart,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{Booted, boot, date};

/// AED in fils.
const fn aed(amount: i64) -> i64 {
    amount * 100
}

/// The project at a contract value of AED 2,000,000.
async fn project_of(t: &Booted, status: ProjectStatus, manager: &User) -> Project {
    let project = t.project(status, Some(manager)).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(t.db(), &[("contract_value", &aed(2_000_000))])
        .await
        .unwrap();
    t.reload(&project).await
}

/// Sets the columns of IPA `id`.
async fn set_ipa(t: &Booted, id: i64, columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))]) {
    Ipa::query()
        .where_(Ipa::ID.eq(id))
        .update(t.db(), columns)
        .await
        .unwrap();
}

#[renox::test]
async fn the_projects_page_lists_the_projects_with_their_reports_and_ipas() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = project_of(&t, ProjectStatus::InProgress, &pm).await;
    let ipa = t
        .ipa(&project, 3, IpaStatus::Submitted, aed(120_000), None)
        .await;
    let stranger = t.user_with("Nadia", "stock_manager").await;

    let page = t.browse(&pm, "/construction/projects").await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains(&project.code), "the code: {html}");
    assert!(html.contains("Marina Tower fit-out"), "{html}");
    assert!(html.contains("In progress"), "the status in words: {html}");
    assert!(html.contains("AED 2,000,000.00"), "the contract value");
    assert!(html.contains("IPA 3"), "the IPA label");
    assert!(html.contains(&format!("/construction/ipas/{}", ipa.id)));
    assert!(html.contains(&format!("/construction/projects/{}/closing", project.id)));
    assert!(html.contains(&format!(
        "/construction/projects/{}/variation-report",
        project.id
    )));

    // Whoever holds the permission in no project has no list.
    t.browse(&stranger, "/construction/projects")
        .await
        .assert_forbidden();
}

#[renox::test]
async fn the_ipa_page_shows_the_amounts_the_deductions_and_the_related_records() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = project_of(&t, ProjectStatus::InProgress, &pm).await;
    let ipa = t
        .ipa(
            &project,
            2,
            IpaStatus::Certified,
            aed(200_000),
            Some(aed(180_000)),
        )
        .await;
    set_ipa(
        &t,
        ipa.id,
        &[
            ("invoice_id", &7_i64),
            ("credit_note_id", &3_i64),
            ("dispute_id", &4_i64),
            ("supplier_bill_id", &5_i64),
            ("certificate_ref", &"CERT-0002"),
        ],
    )
    .await;
    t.deduction(
        &ipa,
        DeductionKind::BackCharge,
        "BC-17",
        aed(12_000),
        DeductionStatus::Accepted,
    )
    .await;

    let path = format!("/construction/ipas/{}", ipa.id);
    let page = t.browse(&pm, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("IPA 2"), "the title: {html}");
    assert!(html.contains("Certified"), "the status");
    assert!(html.contains("AED 200,000.00"), "the claim");
    assert!(html.contains("AED 180,000.00"), "the certified work");
    assert!(html.contains("CERT-0002"), "the certificate");
    assert!(html.contains("BC-17") && html.contains("Back-charge"));
    assert!(html.contains("AED 12,000.00"), "the deduction");
    for link in [
        "/invoices/7",
        "/credit-notes/3",
        "/supplier-bills/5",
        &format!("/construction/projects/{}/closing", project.id),
        &format!("/construction/projects/{}/variation-report", project.id),
    ] {
        assert!(html.contains(link), "a link to {link}: {html}");
    }

    // Without `text/html` the answer is the JSON it was.
    let plain = t.get(&pm, &path).await;
    plain.assert_ok();
    let json: renox::serde_json::Value = renox::serde_json::from_str(&plain.text()).unwrap();
    assert_eq!(json["number"], 2);
    assert_eq!(json["status"], "certified");
    assert_eq!(json["certified_amount"], aed(180_000));
    assert_eq!(json["invoice_id"], 7);
    assert_eq!(json["deductions"][0]["reference"], "BC-17");
    assert!(plain.text().starts_with("{\"id\":"), "{}", plain.text());
}

#[renox::test]
async fn the_closing_page_shows_the_final_account_and_what_is_still_open() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let project = project_of(&t, ProjectStatus::Terminated, &pm).await;
    Project::query()
        .where_(Project::ID.eq(project.id))
        .update(
            t.db(),
            &[
                ("termination_reason", &"terminated_by_client"),
                ("termination_effective_on", &date(2027, 5, 31)),
            ],
        )
        .await
        .unwrap();
    let interim = t
        .ipa(
            &project,
            1,
            IpaStatus::Certified,
            aed(90_000),
            Some(aed(90_000)),
        )
        .await;
    t.deduction(
        &interim,
        DeductionKind::ContraCharge,
        "CC-3",
        aed(12_000),
        DeductionStatus::Contested,
    )
    .await;
    let account = t
        .ipa(
            &project,
            2,
            IpaStatus::Invoiced,
            aed(40_000),
            Some(aed(40_000)),
        )
        .await;
    set_ipa(
        &t,
        account.id,
        &[
            ("kind", &IpaKind::FinalAccount.as_str()),
            ("invoice_id", &11_i64),
        ],
    )
    .await;
    Retention::new(project.id, RetentionPart::Final, aed(50_000))
        .save(t.db())
        .await
        .unwrap();
    let mut guarantee = Guarantee::new(project.id, GuaranteeKind::Performance, aed(20_000));
    guarantee.status = GuaranteeStatus::Active;
    let mut tx = t.db().begin().await.unwrap();
    guarantee
        .insert_in_state(&mut tx, GuaranteeStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let path = format!("/construction/projects/{}/closing", project.id);
    let page = t.browse(&pm, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains(&project.code), "{html}");
    assert!(html.contains("Terminated"), "the status");
    assert!(html.contains("Terminated by the client"), "the reason");
    assert!(html.contains("2027-05-31"), "the last working day");
    assert!(html.contains("Final account"), "the final account card");
    assert!(html.contains("AED 40,000.00"), "its certified work");
    assert!(html.contains("/invoices/11"), "the invoice link");
    assert!(
        html.contains(&format!("/construction/ipas/{}", account.id)),
        "the final account link"
    );
    assert!(html.contains("CC-3") && html.contains("AED 12,000.00"));
    assert!(
        html.contains(&format!("/construction/ipas/{}", interim.id)),
        "the IPA of a contested deduction"
    );
    assert!(html.contains("Still open"), "the warnings: {html}");

    let plain = t.get(&pm, &path).await;
    let json: renox::serde_json::Value = renox::serde_json::from_str(&plain.text()).unwrap();
    assert_eq!(json["status"], "terminated");
    assert_eq!(json["warnings"]["retention_parts"], 1);
    assert_eq!(json["warnings"]["guarantees"], 1);
    assert_eq!(json["contested_deductions"][0]["reference"], "CC-3");
    assert_eq!(json["final_account"]["invoice_id"], 11);
}

#[renox::test]
async fn the_variation_report_page_shows_the_price_the_approval_and_the_gap() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let qs = t.qs("Qasim").await;
    let project = project_of(&t, ProjectStatus::InProgress, &pm).await;
    t.post(
        &qs,
        &format!("/construction/projects/{}/variations", project.id),
        &[
            ("kind", "addition"),
            ("description", "Extra partition on level 3"),
            ("site_instruction", "SI-22"),
            ("priced_amount", "5000000"),
        ],
    )
    .await
    .assert_ok();
    let made = variation::of_project(t.db(), project.id)
        .await
        .unwrap()
        .pop()
        .expect("the variation");
    t.post(
        &pm,
        &format!("/construction/variations/{}/submit", made.id),
        &[],
    )
    .await
    .assert_ok();
    t.post(
        &qs,
        &format!("/construction/variations/{}/approve", made.id),
        &[("client_reference", "VI-9"), ("approved_amount", "4000000")],
    )
    .await
    .assert_ok();

    let path = format!("/construction/projects/{}/variation-report", project.id);
    let page = t.browse(&pm, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("Variation report"), "{html}");
    assert!(html.contains("VO-01"), "the variation");
    assert!(html.contains("Approved"), "the status");
    assert!(html.contains("AED 50,000.00"), "the price");
    assert!(html.contains("AED 40,000.00"), "the approved amount");
    assert!(html.contains("AED 10,000.00"), "the gap");
    assert!(html.contains(&format!("/construction/projects/{}/closing", project.id)));

    let plain = t.get(&pm, &path).await;
    let json: renox::serde_json::Value = renox::serde_json::from_str(&plain.text()).unwrap();
    assert_eq!(json[0]["code"], "VO-01");
    assert_eq!(json[0]["priced_amount"], aed(50_000));
    assert_eq!(json[0]["approved_amount"], aed(40_000));
    assert_eq!(json[0]["gap"], aed(10_000));

    // An empty report says so.
    let other = project_of(&t, ProjectStatus::InProgress, &pm).await;
    let empty = t
        .browse(
            &pm,
            &format!("/construction/projects/{}/variation-report", other.id),
        )
        .await;
    empty.assert_ok();
    assert!(empty.text().contains("No variations yet"));
}

#[renox::test]
async fn a_page_is_refused_to_a_person_who_may_not_read_it() {
    let t = boot().await;
    let pm = t.project_manager("Amal").await;
    let stranger = t.user_with("Nadia", "stock_manager").await;
    let project = project_of(&t, ProjectStatus::InProgress, &pm).await;
    let ipa = t.ipa(&project, 1, IpaStatus::Draft, aed(1), None).await;
    for path in [
        format!("/construction/ipas/{}", ipa.id),
        format!("/construction/projects/{}/closing", project.id),
        format!("/construction/projects/{}/variation-report", project.id),
    ] {
        t.browse(&stranger, &path).await.assert_forbidden();
        t.get(&stranger, &path).await.assert_forbidden();
    }
}
