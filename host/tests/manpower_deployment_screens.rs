//! BL-MPW-24 (#504), `MPW.ACL.07`, `MPW.ACL.14`, `MPW.ACL.15`: the screens of a
//! deployment in the demo agency: the record page with its documents, the
//! checks of *Activate* listed all at once with *Request override* where it is
//! possible, *Extend*, *Suspend* and *Resume* with the days in words, the
//! reasons of *End* with the fields each needs, *Cancel*, *Ask for cover*, and
//! the supervisor who sees the page read-only. Each test opens the page as a
//! role of the demo agency, posts the form of its sheet and checks the words.

mod manpower_screens_support;
mod support;

use manpower::{Deployment, DeploymentStatus};
use manpower_screens_support::{
    AMAL, MD, SUPERVISOR, browse, contract, day, demo, deployment_of, employee, fits_a_phone,
    landed, no_raw_words, page, person, post, says, scalar, visible, words,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;

/// The deployment `id` as it is stored.
async fn stored(app: &TestApp, id: i64) -> Deployment {
    Deployment::find_or_404(app.db(), id).await.unwrap()
}

/// The page of deployment `id`.
fn path(id: i64) -> String {
    format!("/manpower/deployments/{id}")
}

/// The accepted `kind` of the worker called `name` ends on `until`.
async fn document_ends(app: &TestApp, name: &str, kind: &str, until: renox::chrono::NaiveDate) {
    let who = employee(app, name).await;
    let until = until.to_string();
    renox::db::sql(format!(
        "update wl_documents_documents set expiry_date = '{until}' where subject_id = {who} \
         and subject_type = 'hr.employee' and kind_id = \
         (select id from wl_documents_kinds where key = '{kind}')"
    ))
    .execute(app.db())
    .await
    .unwrap();
}

/// A second deployment of Eko, active across the days of the planned one, at
/// the same hours: what makes *Activate* refuse for an overlap.
async fn overlapping_deployment(app: &TestApp, planned: &Deployment) {
    let mut other = Deployment {
        employee_id: planned.employee_id,
        client_contract_id: planned.client_contract_id,
        client_party_id: planned.client_party_id,
        site_id: planned.site_id,
        role: planned.role.clone(),
        shift_type: "day".into(),
        shift_start: Some("07:00".into()),
        shift_end: Some("15:00".into()),
        start_date: day(-5),
        end_date: Some(day(100)),
        status: DeploymentStatus::Active,
        ..Deployment::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    other
        .insert_in_state(&mut tx, DeploymentStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// AC1: W's residence visa expired and W has an overlapping active deployment:
/// *Activate* is refused with both checks as separate lines in words, the
/// deployment stays *Planned*, and the page lists the same two lines.
#[renox::test]
async fn activating_lists_the_expired_visa_and_the_overlap_as_separate_lines() {
    let app = demo().await;
    let id = deployment_of(&app, "Eko Prasetyo").await;
    let planned = stored(&app, id).await;
    assert_eq!(planned.status, DeploymentStatus::Planned);
    document_ends(&app, "Eko Prasetyo", "residence_visa", day(-10)).await;
    overlapping_deployment(&app, &planned).await;

    let refusal = post(&app, AMAL, &format!("{}/activate", path(id)), &[]).await;
    refusal.assert_status(422);
    let body = refusal.text();
    let lines: Vec<&str> = body.lines().collect();
    let visa = lines
        .iter()
        .find(|line| line.contains("Residence visa"))
        .unwrap_or_else(|| panic!("a line for the visa: {body}"));
    assert!(
        visa.contains(&format!("expired on {}", day(-10)))
            && visa.contains("override: not allowed"),
        "{visa}"
    );
    let overlap = lines
        .iter()
        .find(|line| line.contains("already has deployment"))
        .unwrap_or_else(|| panic!("a line for the overlap: {body}"));
    assert!(!overlap.contains("Residence visa"), "{overlap}");
    assert_eq!(stored(&app, id).await.status, DeploymentStatus::Planned);

    let html = page(&app, AMAL, &path(id)).await;
    // The page says the same checks with the worker and the date in words.
    says(
        &html,
        &[
            "Planned",
            "Why it cannot start yet",
            &format!(
                "Residence visa of Eko Prasetyo expired on {} (cannot be overridden)",
                words(day(-10))
            ),
            "already has deployment",
            "Documents",
        ],
    );
    assert!(
        !html.contains("Request override"),
        "the visa cannot be overridden"
    );
    no_raw_words(&html);
}

/// AC2: W's Emirates ID expired: the check line offers *Request override*
/// (the MD approves); the page links the request.
#[renox::test]
async fn an_expired_emirates_id_offers_to_request_an_override() {
    let app = demo().await;
    let id = deployment_of(&app, "Eko Prasetyo").await;
    document_ends(&app, "Eko Prasetyo", "emirates_id", day(-4)).await;

    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Emirates ID of Eko Prasetyo",
            &format!("expired on {}", words(day(-4))),
            "can be overridden with approval",
            "Request override (MD approval)",
            "/documents/overrides/new?kind=emirates_id",
        ],
    );
    no_raw_words(&html);
    let refusal = post(&app, AMAL, &format!("{}/activate", path(id)), &[]).await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("Emirates ID expired on"),
        "{}",
        refusal.text()
    );
}

/// AC3: a clean planned deployment on a contract on hold is refused with *The
/// contract is on hold*; after the release the activation works and the page
/// reads *Active*.
#[renox::test]
async fn a_planned_deployment_waits_for_the_release_of_a_held_contract() {
    let app = demo().await;
    let id = deployment_of(&app, "Eko Prasetyo").await;
    let held = contract(&app, "CC-DEMO-002").await;
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{held}/hold"),
        &[("reason", "other"), ("text", "Client in dispute")],
    )
    .await
    .assert_status(303);

    let refusal = post(&app, AMAL, &format!("{}/activate", path(id)), &[]).await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("The contract is on hold"),
        "{}",
        refusal.text()
    );
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Planned", "The contract is on hold"]);

    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{held}/release"),
        &[],
    )
    .await
    .assert_status(303);
    let activated = post(&app, AMAL, &format!("{}/activate", path(id)), &[]).await;
    activated.assert_status(303);
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Active"]);
    assert_eq!(stored(&app, id).await.status, DeploymentStatus::Active);
}

/// AC4: an active deployment ending before the contract does is extended
/// beyond the day W's work permit ends: refused, naming the work permit and
/// its date; an extension inside the permit works.
#[renox::test]
async fn extending_past_the_work_permit_is_refused_naming_it() {
    let app = demo().await;
    let id = deployment_of(&app, "Ravi Kumar").await;
    // Ravi's work permit ends in 20 days; the deployment ends in 100.
    renox::db::sql(format!(
        "update manpower_deployments set end_date = '{}' where id = {id}",
        day(100)
    ))
    .execute(app.db())
    .await
    .unwrap();
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Extend", "Work permit", "New end date"]);

    let to = day(150).to_string();
    let refusal = post(
        &app,
        AMAL,
        &format!("{}/extend", path(id)),
        &[("end_date", &to)],
    )
    .await;
    refusal.assert_status(422);
    let body = refusal.text();
    assert!(
        body.contains(&format!("Work permit expired on {}", day(20))),
        "{body}"
    );
    assert_eq!(stored(&app, id).await.end_date, Some(day(100)));

    // A permit renewed lets the extension through.
    document_ends(&app, "Ravi Kumar", "work_permit", day(400)).await;
    post(
        &app,
        AMAL,
        &format!("{}/extend", path(id)),
        &[("end_date", &to)],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &[&words(day(150))]);
}

/// AC5: *Suspend* with a reason shows *Suspended* and the days in words
/// (*1 day suspended*, then *3 days suspended* two days on); *Resume* shows
/// *Active* again.
#[renox::test]
async fn suspending_and_resuming_show_the_days_suspended_in_words() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Active", "Suspend"]);
    assert!(
        !html.contains("id=\"resume-deployment\""),
        "an active deployment is not resumed"
    );

    post(
        &app,
        AMAL,
        &format!("{}/suspend", path(id)),
        &[("reason", "Site access withdrawn")],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Suspended",
            "Site access withdrawn",
            "1 day suspended",
            "Resume",
        ],
    );
    assert!(!html.contains("1 days suspended"));
    no_raw_words(&html);
    let list = page(&app, AMAL, "/manpower/deployments").await;
    says(&list, &[&path(id)]);

    app.travel(std::time::Duration::from_secs(2 * 86_400));
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["3 days suspended"]);

    post(&app, AMAL, &format!("{}/resume", path(id)), &[])
        .await
        .assert_status(303);
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Active"]);
    assert_eq!(stored(&app, id).await.status, DeploymentStatus::Active);
}

/// AC6: *End* with *client rejected* and an empty note says *Write the
/// client's note*; with a note the page shows *Ended* and links to the
/// replacement request that opened.
#[renox::test]
async fn ending_for_the_clients_rejection_needs_the_note_and_links_the_replacement() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &["End", "Client rejected the worker", "The client&#39;s note"],
    );

    let empty = post(
        &app,
        AMAL,
        &format!("{}/end", path(id)),
        &[("reason", "client_rejected"), ("note", "")],
    )
    .await;
    empty.assert_status(422);
    assert!(
        empty.text().contains("Write the client's note")
            || empty.text().contains("Write the client&#39;s note"),
        "{}",
        empty.text()
    );
    assert_eq!(stored(&app, id).await.status, DeploymentStatus::Active);

    post(
        &app,
        AMAL,
        &format!("{}/end", path(id)),
        &[("reason", "client_rejected"), ("note", "Late three times")],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Ended",
            "Client rejected the worker",
            "Late three times",
            "Open the replacement request",
        ],
    );
    let replacement = scalar(
        &app,
        &format!(
            "select id from manpower_requests where replaces_deployment_id = {id} \
             and kind = 'replacement'"
        ),
    )
    .await;
    assert!(html.contains(&format!("/manpower/requests/{replacement}\"")));
    no_raw_words(&html);
    let request = page(&app, AMAL, &format!("/manpower/requests/{replacement}")).await;
    says(&request, &["Late three times"]);
}

/// AC7: the account manager is not offered *non-payment* among the reasons and
/// a direct post is refused; the MD sees it and, inside the notice period
/// without a waiver, is refused in words.
#[renox::test]
async fn only_the_md_can_end_for_non_payment_and_a_short_notice_is_refused_in_words() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let for_amal = page(&app, AMAL, &path(id)).await;
    says(
        &for_amal,
        &["Client rejected the worker", "Entered in error"],
    );
    assert!(!for_amal.contains("Client does not pay"));
    let direct = post(
        &app,
        AMAL,
        &format!("{}/end", path(id)),
        &[("reason", "client_non_payment")],
    )
    .await;
    direct.assert_status(403);

    let for_md = page(&app, MD, &path(id)).await;
    says(
        &for_md,
        &["Client does not pay", "Last day on site", "Waiver"],
    );
    let soon = day(3).to_string();
    let refusal = post(
        &app,
        MD,
        &format!("{}/end", path(id)),
        &[("reason", "client_non_payment"), ("end_date", &soon)],
    )
    .await;
    refusal.assert_status(422);
    let body = refusal.text();
    assert!(
        body.contains("notice period") && body.contains("waiver"),
        "{body}"
    );
    assert!(!body.contains("client_non_payment"), "{body}");
    assert_eq!(stored(&app, id).await.status, DeploymentStatus::Active);

    // With the waiver the worker is withdrawn after the last day, said on the page.
    post(
        &app,
        MD,
        &format!("{}/end", path(id)),
        &[
            ("reason", "client_non_payment"),
            ("end_date", &soon),
            ("waiver", "WAIVER-7"),
        ],
    )
    .await
    .assert_status(303);
    let html = page(&app, MD, &path(id)).await;
    says(&html, &[&words(day(3)), "has not paid"]);
}

/// AC8: *Entered in error* shows the deployment as voided with the reason, and
/// the request it filled counts one worker less.
#[renox::test]
async fn a_deployment_entered_in_error_is_voided_and_the_request_count_falls_back() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let request = stored(&app, id).await.request_id.expect("a request");
    let before = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    let headcount = scalar(
        &app,
        &format!("select headcount from manpower_requests where id = {request}"),
    )
    .await;
    let filled = |html: &str| {
        let text = visible(html);
        (0..=headcount).find(|n| text.contains(&format!("{n} of {headcount} filled")))
    };
    let was = filled(&before).expect("the request page says how many are filled");

    post(
        &app,
        AMAL,
        &format!("{}/end", path(id)),
        &[("reason", "entered_in_error")],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Voided", "Entered in error"]);
    assert!(!html.contains("Deployment</span>Ended"));
    no_raw_words(&html);
    let after = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    let now = filled(&after).expect("the request page says how many are filled");
    assert_eq!(now, was - 1, "{was} filled before, {now} after");
}

/// AC9: a planned deployment is cancelled with a reason and shows *Cancelled*;
/// an active one has no *Cancel* button and the route refuses.
#[renox::test]
async fn only_a_planned_deployment_can_be_cancelled() {
    let app = demo().await;
    let active = deployment_of(&app, "Wasim Khan").await;
    let html = page(&app, AMAL, &path(active)).await;
    assert!(!html.contains("id=\"cancel-deployment\""));
    let refusal = post(
        &app,
        AMAL,
        &format!("{}/cancel", path(active)),
        &[("reason", "mistake")],
    )
    .await;
    refusal.assert_status(422);

    let planned = deployment_of(&app, "Eko Prasetyo").await;
    let html = page(&app, AMAL, &path(planned)).await;
    says(&html, &["id=\"cancel-deployment\"", "Activate"]);
    post(
        &app,
        AMAL,
        &format!("{}/cancel", path(planned)),
        &[("reason", "The client changed its mind")],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &path(planned)).await;
    says(&html, &["Cancelled", "The client changed its mind"]);
    assert!(!html.contains("id=\"activate-deployment\""));
}

/// AC10: the site supervisor reads the page and sees no action; a direct post
/// answers 403 with the refusal text.
#[renox::test]
async fn the_supervisor_reads_the_page_and_cannot_act() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let html = page(&app, SUPERVISOR, &path(id)).await;
    says(&html, &["Wasim Khan", "Active", "Documents"]);
    for sheet in ["suspend-deployment", "end-deployment", "extend-deployment"] {
        assert!(!html.contains(sheet), "{sheet} is drawn for the supervisor");
    }
    for action in ["suspend", "extend", "cancel"] {
        let refusal = post(
            &app,
            SUPERVISOR,
            &format!("{}/{action}", path(id)),
            &[("reason", "x"), ("end_date", &day(150).to_string())],
        )
        .await;
        refusal.assert_status(403);
        assert!(
            refusal
                .text()
                .contains("You do not have permission for this action"),
            "{}",
            refusal.text()
        );
    }
    assert_eq!(stored(&app, id).await.status, DeploymentStatus::Active);
}

/// The documents, the timesheets and the activity of a deployment are on its
/// page; the JSON answer keeps working; the contract's link no longer 404s.
#[renox::test]
async fn the_page_shows_the_documents_the_timesheets_and_the_activity() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Wasim Khan",
            "Security guard",
            "Residence visa",
            "SIRA licence",
            "Valid until",
            "Timesheets",
            "Activity",
        ],
    );
    no_raw_words(&html);
    // At 390 px: the tables scroll inside their boxes and the actions are sheets.
    says(&html, &["rx-table-wrap", "rx-sheet", "Regular"]);
    fits_a_phone(&html);
    let contract_page = page(
        &app,
        AMAL,
        &format!(
            "/manpower/contracts/{}",
            contract(&app, "CC-DEMO-001").await
        ),
    )
    .await;
    assert!(contract_page.contains(&format!("{}\"", path(id))));

    let user = person(&app, AMAL).await;
    app.acting_as(&user);
    let json = app.request().json().get(&path(id)).await;
    json.assert_ok();
    assert!(json.text().contains("\"status_key\":\"active\""));
    let gone = browse(&app, AMAL, "/manpower/deployments/999999").await;
    gone.assert_status(404);
}

/// *Ask for cover* opens the cover request for the days given and lands on it;
/// the deployment's page lists it. *Plan a deployment* on the list lands on the
/// page of the new, planned deployment.
#[renox::test]
async fn cover_and_plan_land_on_the_page_they_made() {
    let app = demo().await;
    let id = deployment_of(&app, "Wasim Khan").await;
    let from = day(3).to_string();
    let to = day(9).to_string();
    let asked = post(
        &app,
        AMAL,
        &format!("{}/cover", path(id)),
        &[("from", &from), ("to", &to)],
    )
    .await;
    asked.assert_status(303);
    let request = landed(&asked);
    let html = page(&app, AMAL, &path(id)).await;
    assert!(html.contains(&format!("/manpower/requests/{request}\"")));
    says(&html, &["Cover request", "Ask for cover"]);

    // Plan from the list: the request and the worker only.
    let list = page(&app, AMAL, "/manpower/deployments").await;
    says(&list, &["Plan a deployment", "plan-deployment"]);
    fits_a_phone(&list);
    let open = scalar(
        &app,
        "select min(id) from manpower_requests where role = 'security_guard' \
         and status in ('open', 'partially_filled')",
    )
    .await;
    let worker = employee(&app, "Joseph Mensah").await;
    let (open, worker, start) = (open.to_string(), worker.to_string(), day(30).to_string());
    let planned = post(
        &app,
        AMAL,
        "/manpower/deployments",
        &[
            ("request_id", &open),
            ("employee_id", &worker),
            ("start_date", &start),
        ],
    )
    .await;
    planned.assert_status(303);
    let html = page(&app, AMAL, &path(landed(&planned))).await;
    says(&html, &["Planned", "Joseph Mensah", "Security guard"]);
}
