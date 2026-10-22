//! BL-MPW-28 (#508), `MPW.ACL.01`, `MPW.ACL.02`, `PTY.ACL.02`, `PTY.ACL.03`:
//! the client approver's portal in the demo agency: the nav of a portal-only
//! person, the timesheets waiting for him, approving and rejecting with the
//! page of `/timesheets/{id}`, the other client's records answering 404, the
//! viewer who only sees, the staff URLs that answer 403, the worker pages
//! without pay or documents, the blocked client and the contract on hold with
//! their neutral words, and the phone structure. Each test opens the page as a
//! person of the demo agency and checks the words on the page.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    AMAL, browse, client, contract, day, demo, employee, fits_a_phone, no_raw_words, page, person,
    post, says, scalar, visible, words,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_portal::{AccessGrant, GrantStatus};

/// Carl Whitfield, the client approver of Gulf Tower.
const CARL: &str = "approver@gulftower.example";
/// The neutral text of a blocked client.
const NEUTRAL: &str = "This request cannot be completed online. Please contact us.";

/// The first timesheet of the worker called `name`.
async fn sheet_of(app: &TestApp, name: &str) -> i64 {
    let who = employee(app, name).await;
    scalar(
        app,
        &format!(
            "select min(id) from wl_attendance_timesheets where owner_employee_id = {who} \
             and kind = 'regular'"
        ),
    )
    .await
}

/// The timesheet `id` as it is stored.
async fn stored(app: &TestApp, id: i64) -> Timesheet {
    Timesheet::find_or_404(app.db(), id).await.unwrap()
}

/// A user of the client company `company`, a person of the portal with the
/// portal roles `roles` (`approver` or `viewer`); the approver also holds the
/// role the demo's timesheets name as their external approver.
async fn portal_user(
    app: &TestApp,
    email: &str,
    person_name: &str,
    company: &str,
    roles: &str,
) -> String {
    let mut user = User::register(app.db(), person_name, email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    user.assign_role(app.db(), host::roles::MEMBER)
        .await
        .unwrap();
    if roles == "approver" {
        user.assign_role(app.db(), "client_approver").await.unwrap();
    }
    let person_party = client(app, person_name).await;
    let company = client(app, company).await;
    let mut grant = AccessGrant {
        user_id: Some(user.id),
        person_party_id: person_party.0,
        party_id: company.0,
        audience: "tenant".into(),
        roles: roles.into(),
        source: "invitation".into(),
        valid_from: day(-30),
        status: GrantStatus::Active,
        ..AccessGrant::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    grant
        .insert_in_state(&mut tx, GrantStatus::Active, "test")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    email.to_owned()
}

/// Marina Bay's approver (Layla Rashed).
async fn layla(app: &TestApp) -> String {
    portal_user(
        app,
        "layla@marinabay.example",
        "Layla Rashed",
        "Marina Bay Hospitality LLC",
        "approver",
    )
    .await
}

/// A viewer of Gulf Tower (Fiona Mathew).
async fn viewer(app: &TestApp) -> String {
    portal_user(
        app,
        "viewer@gulftower.example",
        "Fiona Mathew",
        "Gulf Tower Facilities LLC",
        "viewer",
    )
    .await
}

/// AC1 and AC2 and the phone: the approver's home says *2 timesheets waiting for
/// you* and links to the list; his sidebar has *Requests*, *Workers on site* and
/// *Timesheets* and no staff link (*Review tasks*, *Broadcasts*).
#[renox::test]
async fn the_approvers_nav_is_the_portals_and_the_home_counts_his_timesheets() {
    let app = demo().await;
    // Gulf Tower has one sheet waiting; the agency submits Wasim's: two.
    let one = page(&app, CARL, "/dashboard").await;
    says(&one, &["1 timesheet waiting for you"]);
    let wasim = sheet_of(&app, "Wasim Khan").await;
    post(&app, AMAL, &format!("/timesheets/{wasim}/submit"), &[])
        .await
        .assert_ok();
    let html = page(&app, CARL, "/dashboard").await;
    says(
        &html,
        &[
            "2 timesheets waiting for you",
            "href=\"/portal/manpower/timesheets\"",
            "Requests",
            "Workers on site",
            "Timesheets",
            "href=\"/portal/manpower/requests\"",
            "href=\"/portal/manpower/deployments\"",
        ],
    );
    for staff in [
        "Review tasks",
        "Broadcasts",
        "Block overrides",
        "SOP acknowledgements",
        "Parties",
    ] {
        assert!(!html.contains(staff), "the portal user is shown `{staff}`");
    }
    // The staff still have theirs.
    let staff = page(&app, "admin@desertstar.example", "/dashboard").await;
    says(&staff, &["Review tasks", "Broadcasts"]);

    let list = page(&app, CARL, "/portal/manpower/timesheets").await;
    says(
        &list,
        &[
            "2 timesheets waiting for you",
            "Ravi Kumar",
            "Wasim Khan",
            "Joseph Mensah",
            "Waiting for you",
            "Submitted",
            "Approved",
            "href=\"/timesheets/",
        ],
    );
    assert!(
        !list.contains("Bilal Ahmed"),
        "Bilal works for Marina Bay, not for Gulf Tower"
    );
    no_raw_words(&list);
    fits_a_phone(&list);
}

/// AC3: the approver opens a Gulf Tower sheet from the list, approves it and the
/// page shows *Approved* with his name and the date; rejecting another without a
/// reason is refused in words, with one it goes back to the agency.
#[renox::test]
async fn the_approver_approves_a_sheet_and_cannot_reject_without_a_reason() {
    let app = demo().await;
    let ravi = sheet_of(&app, "Ravi Kumar").await;
    let before = page(&app, CARL, &format!("/timesheets/{ravi}")).await;
    says(
        &before,
        &[
            "Timesheet of Ravi Kumar",
            "Submitted",
            "id=\"approve-timesheet\"",
            "id=\"reject-timesheet\"",
            "/portal/manpower/timesheets",
            "10.00 h",
        ],
    );
    assert!(
        !before.contains("href=\"/timesheets\""),
        "back goes to the portal list"
    );
    assert!(!before.contains("/invoices/"), "no staff link on the page");
    fits_a_phone(&before);
    no_raw_words(&before);

    post(&app, CARL, &format!("/timesheets/{ravi}/approve"), &[])
        .await
        .assert_ok();
    assert_eq!(stored(&app, ravi).await.status, TimesheetStatus::Approved);
    let after = page(&app, CARL, &format!("/timesheets/{ravi}")).await;
    says(
        &after,
        &["Approved", "Carl Whitfield", &words(day(0)).to_string()],
    );
    assert!(!after.contains("id=\"approve-timesheet\""));

    // The agency submits Wasim's sheet; the approver sends it back.
    let bilal = sheet_of(&app, "Wasim Khan").await;
    post(&app, AMAL, &format!("/timesheets/{bilal}/submit"), &[])
        .await
        .assert_ok();
    let refused = post(
        &app,
        CARL,
        &format!("/timesheets/{bilal}/reject"),
        &[("reason", " ")],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Write the reason"),
        "{}",
        refused.text()
    );
    assert_eq!(stored(&app, bilal).await.status, TimesheetStatus::Submitted);
    post(
        &app,
        CARL,
        &format!("/timesheets/{bilal}/reject"),
        &[("reason", "Two days are wrong")],
    )
    .await
    .assert_ok();
    assert_eq!(stored(&app, bilal).await.status, TimesheetStatus::Open);

    let home = page(&app, CARL, "/dashboard").await;
    assert!(
        !home.contains("timesheets waiting for you") && !home.contains("timesheet waiting for you"),
        "nothing waits for him now"
    );
}

/// AC4: the approver of another client opens or approves a Gulf Tower sheet and
/// gets 404, nothing changes; the Gulf Tower approver meets 404 on the sheet of
/// Al Quoz; each sees only his own company's rows.
#[renox::test]
async fn a_sheet_or_request_of_another_client_answers_not_found() {
    let app = demo().await;
    let layla = layla(&app).await;
    let ravi = sheet_of(&app, "Ravi Kumar").await;
    let path = format!("/timesheets/{ravi}");
    browse(&app, &layla, &path).await.assert_status(404);
    post(&app, &layla, &format!("{path}/approve"), &[])
        .await
        .assert_status(404);
    post(&app, &layla, &format!("{path}/reject"), &[("reason", "no")])
        .await
        .assert_status(404);
    assert_eq!(stored(&app, ravi).await.status, TimesheetStatus::Submitted);

    let billed = scalar(
        &app,
        "select min(id) from wl_attendance_timesheets where status = 'locked'",
    )
    .await;
    browse(&app, CARL, &format!("/timesheets/{billed}"))
        .await
        .assert_status(404);

    // Marina's approver sees Marina and nothing of Gulf Tower; Carl the reverse.
    let marina = visible(&page(&app, &layla, "/portal/manpower/requests").await);
    assert!(marina.contains("Marina Bay Hospitality LLC"), "{marina}");
    assert!(!marina.contains("Gulf Tower"), "{marina}");
    let tower = visible(&page(&app, CARL, "/portal/manpower/requests").await);
    assert!(tower.contains("Gulf Tower Facilities LLC"));
    assert!(!tower.contains("Marina Bay"), "{tower}");
    let workers = visible(&page(&app, &layla, "/portal/manpower/deployments").await);
    assert!(workers.contains("Bilal Ahmed"));
    assert!(!workers.contains("Wasim Khan") && !workers.contains("Ravi Kumar"));
    let sheets = visible(&page(&app, &layla, "/portal/manpower/timesheets").await);
    assert!(sheets.contains("Bilal Ahmed") && sheets.contains("1 timesheet waiting for you"));
    assert!(!sheets.contains("Ravi Kumar"), "{sheets}");

    // A request of Gulf Tower cannot be submitted or cancelled from Marina.
    let request = scalar(
        &app,
        "select min(id) from manpower_requests where status = 'draft'",
    )
    .await;
    if request > 0 {
        post(
            &app,
            &layla,
            &format!("/portal/manpower/requests/{request}/submit"),
            &[],
        )
        .await
        .assert_status(404);
    }
    // The section of the portal framework: only for the company he acts for.
    let tower_party = client(&app, "Gulf Tower Facilities LLC").await;
    let user = person(&app, &layla).await;
    app.acting_as(&user);
    app.request()
        .get(&format!("/portal/{}/manpower_requests", tower_party.0))
        .await
        .assert_status(403);
}

/// AC5: a viewer of Gulf Tower sees the same pages with data and no button; his
/// POST answers 403 with the refusal text.
#[renox::test]
async fn a_viewer_sees_the_data_and_no_button_and_cannot_post() {
    let app = demo().await;
    let viewer = viewer(&app).await;
    let ravi = sheet_of(&app, "Ravi Kumar").await;
    let sheet = page(&app, &viewer, &format!("/timesheets/{ravi}")).await;
    says(&sheet, &["Timesheet of Ravi Kumar", "Submitted", "10.00 h"]);
    for button in [
        "approve-timesheet",
        "reject-timesheet",
        "on-behalf-timesheet",
    ] {
        assert!(!sheet.contains(button), "a viewer is offered `{button}`");
    }
    let requests = page(&app, &viewer, "/portal/manpower/requests").await;
    says(&requests, &["Requests", "Security guard", "CC-DEMO-001"]);
    for button in [
        "portal-new-request",
        "portal-submit-request",
        "portal-cancel-request",
    ] {
        assert!(!requests.contains(button), "a viewer is offered `{button}`");
    }
    let list = page(&app, &viewer, "/portal/manpower/timesheets").await;
    says(&list, &["Ravi Kumar", "Submitted"]);
    assert!(
        !list.contains("waiting for you"),
        "nothing waits for a viewer"
    );
    page(&app, &viewer, "/portal/manpower/deployments").await;

    let refused = post(&app, &viewer, &format!("/timesheets/{ravi}/approve"), &[]).await;
    refused.assert_status(403);
    assert!(
        refused
            .text()
            .contains("You do not have permission for this action."),
        "{}",
        refused.text()
    );
    assert_eq!(stored(&app, ravi).await.status, TimesheetStatus::Submitted);
    post(
        &app,
        &viewer,
        "/portal/manpower/requests",
        &[("client_contract_id", "1")],
    )
    .await
    .assert_status(403);
}

/// AC6: the approver cannot open the staff pages.
#[renox::test]
async fn the_approver_is_refused_the_staff_pages() {
    let app = demo().await;
    for path in [
        "/manpower/contracts",
        "/hr/employees",
        "/timesheets",
        "/parties",
        "/approvals/1",
    ] {
        let response = browse(&app, CARL, path).await;
        let status = response.status.as_u16();
        assert!(
            status == 403 || status == 404,
            "{path} answers {status} to the client approver"
        );
    }
    browse(&app, CARL, "/manpower/contracts")
        .await
        .assert_status(403);
    browse(&app, CARL, "/hr/employees").await.assert_status(403);
    browse(&app, CARL, "/timesheets").await.assert_status(403);
}

/// AC7 and the phone: the workers page says name, role, site, shift, dates and
/// status, and nothing of pay, bank, documents, nationality or health.
#[renox::test]
async fn the_worker_pages_show_no_pay_documents_or_health() {
    let app = demo().await;
    let html = page(&app, CARL, "/portal/manpower/deployments").await;
    says(
        &html,
        &[
            "Workers on site",
            "Wasim Khan",
            "Security guard",
            "Active",
            "Day shift",
        ],
    );
    let text = visible(&html).to_lowercase();
    for secret in [
        "salary",
        "wage",
        "bank",
        "iban",
        "passport",
        "visa",
        "nationality",
        "medical",
        "fitness",
        "emirates id",
        "aed",
        "rate",
    ] {
        assert!(!text.contains(secret), "the workers page shows `{secret}`");
    }
    assert!(!text.contains("al quoz"), "no other client");
    no_raw_words(&html);
    fits_a_phone(&html);
    // The sheet page of a worker: hours, no pay.
    let ravi = sheet_of(&app, "Ravi Kumar").await;
    let sheet = visible(&page(&app, CARL, &format!("/timesheets/{ravi}")).await).to_lowercase();
    for secret in ["salary", "wage", "aed", "iban", "passport"] {
        assert!(!sheet.contains(secret), "the sheet shows `{secret}`");
    }
}

/// The approver orders: *New request* makes a draft, *Submit* sends it, *Cancel
/// request* closes it; the page says each in words and lands back on the portal.
#[renox::test]
async fn the_approver_orders_submits_and_cancels_a_request() {
    let app = demo().await;
    let contract_id = contract(&app, "CC-DEMO-001").await;
    let site = scalar(
        &app,
        &format!(
            "select s.id from manpower_sites s join manpower_client_contracts c \
             on c.client_party_id = s.client_party_id where c.id = {contract_id} order by s.id limit 1"
        ),
    )
    .await;
    let before = page(&app, CARL, "/portal/manpower/requests").await;
    says(
        &before,
        &["New request", "id=\"portal-new-request\"", "Active"],
    );
    let start = day(10).to_string();
    let end = day(90).to_string();
    let created = post(
        &app,
        CARL,
        "/portal/manpower/requests",
        &[
            ("client_contract_id", &contract_id.to_string()),
            ("site_id", &site.to_string()),
            ("role", "Cleaner"),
            ("headcount", "3"),
            ("shift_type", "day"),
            ("start_date", &start),
            ("end_date", &end),
        ],
    )
    .await;
    created.assert_status(303);
    assert_eq!(
        created.header("location"),
        Some("/portal/manpower/requests")
    );
    let draft = page(&app, CARL, "/portal/manpower/requests").await;
    says(
        &draft,
        &["Cleaner", "Draft", "0 of 3 filled, 3 missing", "Submit"],
    );
    no_raw_words(&draft);
    let id = scalar(&app, "select max(id) from manpower_requests").await;
    post(
        &app,
        CARL,
        &format!("/portal/manpower/requests/{id}/submit"),
        &[],
    )
    .await
    .assert_status(303);
    says(
        &page(&app, CARL, "/portal/manpower/requests").await,
        &["Submitted", "Cancel request"],
    );
    post(
        &app,
        CARL,
        &format!("/portal/manpower/requests/{id}/cancel"),
        &[("reason", "Plans changed")],
    )
    .await
    .assert_status(303);
    says(
        &page(&app, CARL, "/portal/manpower/requests").await,
        &["Cancelled"],
    );
    fits_a_phone(&page(&app, CARL, "/portal/manpower/requests").await);
}

/// AC8: with Gulf Tower blocked the approver sees the neutral text and no *New
/// request*, a direct POST is refused with the same text, and nothing of the
/// block, its reason or its note reaches the page.
#[renox::test]
async fn a_blocked_client_sees_the_neutral_text_and_no_new_request() {
    let app = demo().await;
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    support::block(&app, tower).await;
    // The compliance officer's own toast of the block is shown on the next page
    // of the test client; let it go before looking at what Carl is shown.
    browse(&app, CARL, "/dashboard").await;
    let html = page(&app, CARL, "/portal/manpower/requests").await;
    says(&html, &[NEUTRAL]);
    assert!(!html.contains("portal-new-request"), "no New request");
    let text = visible(&html).to_lowercase();
    for leak in [
        "blocked",
        "non_payment",
        "non payment",
        "bounced cheque",
        "on hold",
    ] {
        assert!(
            !text.contains(leak),
            "the page leaks `{leak}`: {}",
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
    let contract_id = contract(&app, "CC-DEMO-001").await;
    let refused = post(
        &app,
        CARL,
        "/portal/manpower/requests",
        &[
            ("client_contract_id", &contract_id.to_string()),
            ("site_id", "1"),
            ("role", "Cleaner"),
            ("headcount", "1"),
            ("shift_type", "day"),
            ("start_date", &day(10).to_string()),
        ],
    )
    .await;
    refused.assert_status(422);
    assert!(refused.text().contains("cannot be completed online"));
    assert!(!refused.text().to_lowercase().contains("non_payment"));
    // The workers on site and the timesheets stay readable, and he can still approve.
    page(&app, CARL, "/portal/manpower/deployments").await;
    let ravi = sheet_of(&app, "Ravi Kumar").await;
    post(&app, CARL, &format!("/timesheets/{ravi}/approve"), &[])
        .await
        .assert_ok();
}

/// A contract on credit hold reads *On hold* to the approver and nothing more:
/// not the reason, the cheque, the state before or who held it; *Submit* is
/// disabled with the neutral line.
#[renox::test]
async fn a_contract_on_hold_reads_on_hold_and_no_reason() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{id}/hold"),
        &[("reason", "bounced_cheque"), ("text", "CHQ-9")],
    )
    .await
    .assert_status(303);
    let ravi = sheet_of(&app, "Ravi Kumar").await;
    for path in [
        "/portal/manpower/requests".to_owned(),
        "/portal/manpower/timesheets".to_owned(),
        "/portal/manpower/deployments".to_owned(),
        format!("/timesheets/{ravi}"),
        "/dashboard".to_owned(),
    ] {
        let html = page(&app, CARL, &path).await;
        let text = visible(&html).to_lowercase();
        for leak in [
            "bounced",
            "cheque",
            "chq-9",
            "overdue",
            "amal youssef",
            "hold reason",
        ] {
            assert!(!text.contains(leak), "{path} leaks `{leak}`");
        }
        no_raw_words(&html);
    }
    let html = page(&app, CARL, "/portal/manpower/requests").await;
    says(
        &html,
        &["On hold", "The contract is on hold. New requests wait."],
    );
}

/// The framework's section page lists the same rows in words for the company
/// the person acts for.
#[renox::test]
async fn the_portal_sections_list_the_companys_rows() {
    let app = demo().await;
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    for (section, words) in [
        ("manpower_requests", "Security guard"),
        ("manpower_deployments", "Wasim Khan"),
        ("manpower_timesheets", "Ravi Kumar"),
    ] {
        let html = page(&app, CARL, &format!("/portal/{}/{section}", tower.0)).await;
        says(&html, &[words]);
        no_raw_words(&html);
    }
}

/// The account manager holds roles scoped to his clients only: he is staff,
/// not a portal person, so his sidebar has the approvals and the review
/// tasks and not the client's own pages.
#[renox::test]
async fn the_account_manager_is_not_taken_for_a_portal_person() {
    let app = demo().await;
    let html = page(&app, AMAL, "/dashboard").await;
    says(&html, &["href=\"/approvals\"", "Review tasks"]);
    assert!(!html.contains("href=\"/portal/manpower/requests\""));
    assert!(!html.contains("href=\"/portal/manpower/timesheets\""));
    // The client approver keeps his own links and has no staff ones.
    let carl = page(&app, CARL, "/dashboard").await;
    says(&carl, &["href=\"/portal/manpower/requests\""]);
    assert!(!carl.contains("Review tasks"));
}
