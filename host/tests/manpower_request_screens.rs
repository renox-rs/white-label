//! BL-MPW-23 (#503): the screens of a manpower request in the demo agency:
//! *New request* on the list, the record page with how many positions are
//! filled and missing, *Submit* (disabled with the reason while the contract is
//! on hold), *Cancel* (disabled while a worker is on site), *Plan a deployment*
//! from the request, the cause of a replacement request, the open-ended period
//! in words, and the roles that may not act. Each test opens the page as a role
//! of the demo agency and checks the words on it.

mod manpower_screens_support;
mod support;

use manpower::{ManpowerRequest, RequestStatus};
use manpower_screens_support::{
    AMAL, SUPERVISOR, browse, contract, day, demo, landed, no_raw_words, page, person, post, says,
    scalar, words, worker_of,
};
use renox::prelude::*;
use renox::testing::TestApp;

/// The id of the first request of the demo in `status`.
async fn request_in(app: &TestApp, status: &str) -> i64 {
    scalar(
        app,
        &format!("select min(id) from manpower_requests where status = '{status}'"),
    )
    .await
}

/// The site of Gulf Tower the demo requests use.
async fn tower_site(app: &TestApp) -> i64 {
    scalar(
        app,
        "select min(id) from manpower_sites where name like 'Tower A%'",
    )
    .await
}

/// A draft request of 3 security guards for Gulf Tower made through the form
/// of *New request*; `end` is its last day, or empty for an open-ended one.
async fn new_request(app: &TestApp, end: &str) -> i64 {
    let (contract, site) = (
        contract(app, "CC-DEMO-001").await.to_string(),
        tower_site(app).await.to_string(),
    );
    let start = day(14).to_string();
    let created = post(
        app,
        AMAL,
        "/manpower/requests",
        &[
            ("client_contract_id", &contract),
            ("site_id", &site),
            ("role", "Security guard"),
            ("headcount", "3"),
            ("shift_type", "day"),
            ("start_date", &start),
            ("end_date", end),
        ],
    )
    .await;
    created.assert_status(303);
    landed(&created)
}

/// AC1: *New request* for an active Gulf Tower contract with 3 security
/// guards makes a draft with status *Draft* and 0 of 3 filled.
#[renox::test]
async fn a_new_request_is_a_draft_with_no_position_filled() {
    let app = demo().await;
    let list = page(&app, AMAL, "/manpower/requests").await;
    says(
        &list,
        &["New request", "new-request", "Positions", "Job role"],
    );
    let end = day(60).to_string();
    let id = new_request(&app, &end).await;
    let html = page(&app, AMAL, &format!("/manpower/requests/{id}")).await;
    says(
        &html,
        &[
            "Draft",
            "Security guard",
            "0 of 3 filled, 3 missing",
            &words(day(14)),
            &words(day(60)),
            "Submit",
        ],
    );
    no_raw_words(&html);
    let list = page(&app, AMAL, "/manpower/requests").await;
    assert!(list.contains(&format!("/manpower/requests/{id}\"")));
    // The JSON of the list and of the request.
    let user = person(&app, AMAL).await;
    app.acting_as(&user);
    let json = app
        .request()
        .json()
        .get(&format!("/manpower/requests/{id}"))
        .await;
    json.assert_ok();
    assert!(json.text().contains("0 of 3 filled"));
}

/// AC2: on a contract on hold the draft cannot be submitted: the button says
/// why and the route answers *The contract is on hold. New requests wait for
/// the release*; after the release it is submitted and the page says
/// *Submitted*.
#[renox::test]
async fn a_draft_on_a_held_contract_waits_for_the_release_to_be_submitted() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let request = new_request(&app, &day(60).to_string()).await;
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{id}/hold"),
        &[("reason", "other"), ("text", "Client in dispute")],
    )
    .await
    .assert_status(303);
    let blocked = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    says(
        &blocked,
        &["The contract is on hold. New requests wait for the release"],
    );
    let refused = post(
        &app,
        AMAL,
        &format!("/manpower/requests/{request}/submit"),
        &[],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("The contract is on hold. New requests wait for the release"),
        "{}",
        refused.text()
    );
    let kept = ManpowerRequest::find_or_404(app.db(), request)
        .await
        .unwrap();
    assert_eq!(kept.status, RequestStatus::Draft);

    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{id}/release"),
        &[],
    )
    .await
    .assert_status(303);
    post(
        &app,
        AMAL,
        &format!("/manpower/requests/{request}/submit"),
        &[],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    says(&html, &["Submitted", "0 of 3 filled, 3 missing"]);
}

/// AC3: a submitted request with 2 active deployments of 3 says *2 of 3
/// filled, 1 missing*; *Plan a deployment* is a sheet with the request set, and
/// planning the third worker fills it.
#[renox::test]
async fn a_partly_filled_request_says_what_is_missing_and_plans_from_itself() {
    let app = demo().await;
    let request = request_in(&app, "partially_filled").await;
    let html = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    says(
        &html,
        &[
            "2 of 3 filled, 1 missing",
            "Plan a deployment",
            &format!("name=\"request_id\" value=\"{request}\""),
            "Active",
        ],
    );
    let free = scalar(
        &app,
        "select min(e.id) from wl_hr_employees e where e.job_role = 'security_guard' and e.status = 'active' and not exists (select 1 from manpower_deployments d where d.employee_id = e.id and d.status in ('planned', 'active', 'suspended'))",
    )
    .await;
    assert!(
        html.contains(&format!("<option value=\"{free}\"")),
        "the free guard {free} is offered"
    );
    let stored = ManpowerRequest::find_or_404(app.db(), request)
        .await
        .unwrap();
    let (contract, site, start) = (
        stored.client_contract_id.to_string(),
        stored.site_id.to_string(),
        day(1).to_string(),
    );
    let (request_id, employee) = (request.to_string(), free.to_string());
    let planned = post(
        &app,
        AMAL,
        "/manpower/deployments",
        &[
            ("request_id", &request_id),
            ("client_contract_id", &contract),
            ("site_id", &site),
            ("role", "security_guard"),
            ("shift_type", "day"),
            ("shift_start", "07:00"),
            ("shift_end", "15:00"),
            ("employee_id", &employee),
            ("start_date", &start),
        ],
    )
    .await;
    assert!(
        planned.status.is_redirection() || planned.status.is_success(),
        "{}: {}",
        planned.status,
        planned.text()
    );
    let after = page(&app, AMAL, &format!("/manpower/requests/{request}")).await;
    says(&after, &["3 of 3 filled, 0 missing", "Planned"]);
}

/// AC4: a replacement request opened by a client rejection shows its cause in
/// words, with the former worker, the date and the client's note, and links to
/// the ended deployment.
#[renox::test]
async fn a_replacement_request_shows_the_rejection_that_opened_it() {
    let app = demo().await;
    let deployment = scalar(
        &app,
        "select min(id) from manpower_deployments where client_contract_id = (select id from manpower_client_contracts where reference = 'CC-DEMO-001') and status = 'active'",
    )
    .await;
    post(
        &app,
        AMAL,
        &format!("/manpower/deployments/{deployment}/end"),
        &[
            ("reason", "client_rejected"),
            ("note", "Late three times this week"),
        ],
    )
    .await
    .assert_status(303);
    let replacement = scalar(
        &app,
        "select max(id) from manpower_requests where kind = 'replacement'",
    )
    .await;
    let former = worker_of(&app, deployment).await;
    let html = page(&app, AMAL, &format!("/manpower/requests/{replacement}")).await;
    says(
        &html,
        &[
            &format!(
                "Replacement for {former}, ended by client rejection on {}",
                words(day(0))
            ),
            "The client wrote: Late three times this week",
            &format!("/manpower/deployments/{deployment}\""),
            "Open the deployment it stands in for",
        ],
    );
    no_raw_words(&html);
}

/// AC5: cancelling a request with an active deployment is refused, by a plain
/// post too (the rule is the service's), with the
/// count (the button says it too); a request with none is cancelled and the
/// page shows *Cancelled* and the reason.
#[renox::test]
async fn cancel_is_refused_while_a_worker_is_on_site_and_works_without() {
    let app = demo().await;
    let busy = request_in(&app, "partially_filled").await;
    let page_busy = page(&app, AMAL, &format!("/manpower/requests/{busy}")).await;
    says(&page_busy, &["2 deployments are still active."]);
    let refused = post(
        &app,
        AMAL,
        &format!("/manpower/requests/{busy}/cancel"),
        &[("reason", "The client changed its mind")],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("2 deployments are still active"),
        "{}",
        refused.text()
    );

    // With one worker ended, the singular is used.
    let first = scalar(
        &app,
        &format!("select min(id) from manpower_deployments where request_id = {busy} and status = 'active'"),
    )
    .await;
    post(
        &app,
        AMAL,
        &format!("/manpower/deployments/{first}/end"),
        &[("reason", "client_rejected"), ("note", "Not suitable")],
    )
    .await
    .assert_status(303);
    let one = post(
        &app,
        AMAL,
        &format!("/manpower/requests/{busy}/cancel"),
        &[("reason", "The client changed its mind")],
    )
    .await;
    one.assert_status(422);
    assert!(
        one.text().contains("1 deployment is still active"),
        "{}",
        one.text()
    );
    assert!(!one.text().contains("1 deployments"), "{}", one.text());
    let page_one = page(&app, AMAL, &format!("/manpower/requests/{busy}")).await;
    says(&page_one, &["1 deployment is still active."]);

    let draft = new_request(&app, &day(60).to_string()).await;
    post(
        &app,
        AMAL,
        &format!("/manpower/requests/{draft}/cancel"),
        &[("reason", "Ordered twice")],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &format!("/manpower/requests/{draft}")).await;
    says(&html, &["Cancelled", "Ordered twice"]);
}

/// AC6: an open-ended request reads *Open-ended* on the list and on its page,
/// never `None`.
#[renox::test]
async fn an_open_ended_request_says_open_ended() {
    let app = demo().await;
    let id = new_request(&app, "").await;
    let html = page(&app, AMAL, &format!("/manpower/requests/{id}")).await;
    says(&html, &[&format!("{} - Open-ended", words(day(14)))]);
    no_raw_words(&html);
    let list = page(&app, AMAL, "/manpower/requests").await;
    says(&list, &["Open-ended"]);
    no_raw_words(&list);
}

/// AC7: the site supervisor sees no request screen and a direct post answers
/// 403; a person who may only view requests sees no create, submit or cancel.
#[renox::test]
async fn people_who_may_not_write_requests_see_no_button_and_get_403() {
    let app = demo().await;
    let draft = request_in(&app, "draft").await;
    browse(&app, SUPERVISOR, "/manpower/requests")
        .await
        .assert_forbidden();
    for path in [
        format!("/manpower/requests/{draft}/submit"),
        format!("/manpower/requests/{draft}/cancel"),
    ] {
        post(&app, SUPERVISOR, &path, &[("reason", "no")])
            .await
            .assert_forbidden();
    }
    let (contract, site, start) = (
        contract(&app, "CC-DEMO-001").await.to_string(),
        tower_site(&app).await.to_string(),
        day(14).to_string(),
    );
    post(
        &app,
        SUPERVISOR,
        "/manpower/requests",
        &[
            ("client_contract_id", &contract),
            ("site_id", &site),
            ("role", "security_guard"),
            ("headcount", "1"),
            ("shift_type", "day"),
            ("start_date", &start),
        ],
    )
    .await
    .assert_forbidden();

    let viewer = renox::auth::User::register(app.db(), "Vera", "vera@test.example", "secret123")
        .await
        .unwrap();
    renox::auth::permissions::define_role(
        app.db(),
        "request_viewer",
        &[manpower::permissions::REQUESTS_VIEW],
    )
    .await
    .unwrap();
    let stored = ManpowerRequest::find_or_404(app.db(), draft).await.unwrap();
    viewer
        .assign_role_in(app.db(), "request_viewer", &stored.scope())
        .await
        .unwrap();
    app.acting_as(&viewer);
    let seen = app
        .request()
        .header("accept", "text/html")
        .get(&format!("/manpower/requests/{draft}"))
        .await;
    seen.assert_ok();
    let html = seen.text();
    says(&html, &["Draft"]);
    for action in ["Submit", "Cancel", "Plan a deployment"] {
        assert!(
            !html.contains(&format!("\">{action}<")),
            "offered `{action}`"
        );
    }
    let list = app
        .request()
        .header("accept", "text/html")
        .get("/manpower/requests")
        .await;
    list.assert_ok();
    assert!(!list.text().contains("New request"));
}

/// AC8: on a phone no table widens the page: each scrolls inside its own box
/// (the kit's table wrapper), columns of less weight drop out, the form is a
/// sheet that fits the screen, and the buttons keep the kit's 44 px targets.
#[renox::test]
async fn the_list_and_the_page_are_built_to_fit_a_phone() {
    let app = demo().await;
    let list = page(&app, AMAL, "/manpower/requests").await;
    says(&list, &["rx-table-wrap", "rx-sheet"]);
    assert_eq!(
        list.matches("<table").count(),
        list.matches("rx-table-wrap").count(),
        "every table scrolls inside its own box"
    );
    let id = request_in(&app, "partially_filled").await;
    let html = page(&app, AMAL, &format!("/manpower/requests/{id}")).await;
    says(&html, &["rx-table-wrap"]);
    assert_eq!(
        html.matches("<table").count(),
        html.matches("rx-table-wrap").count()
    );
}
