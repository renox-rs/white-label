//! BL-MPW-27 (#507), `MPW.ACL.09`, `HRM.ACL.08`, `HRM.RUL.15`: the timesheet
//! screens in the demo agency: the list with the client, the worker, the month,
//! the hours and the status in words; the page with the hours per day and
//! category and the history; *Submit* refused with the two hour figures;
//! *Escalated* and *Deemed approved* with their dates and the clause; *Reject*
//! with a reason; *Approve on behalf* with the signed sheet; the adjustment its
//! creator cannot approve; the billed sheet with its invoice. Each test opens
//! the page as a role of the demo agency, posts the form of its sheet and checks
//! the words.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    ADMIN, AMAL, HR, SUPERVISOR, browse, cells_of_row, day, demo, employee, fits_a_phone,
    no_raw_words, page, person, post, says, scalar, visible, words,
};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_attendance::escalation;
use wl_attendance::{Timesheet, TimesheetEntry, TimesheetStatus};

/// The first timesheet of the worker called `name` in the demo.
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

/// The page of timesheet `id`.
fn path(id: i64) -> String {
    format!("/timesheets/{id}")
}

/// The timesheet `id` as it is stored.
async fn stored(app: &TestApp, id: i64) -> Timesheet {
    Timesheet::find_or_404(app.db(), id).await.unwrap()
}

/// AC1 and the phone: the account manager opens `/timesheets` and the demo
/// timesheets are listed as HTML with the client, the worker, the month, the
/// hours and the status in words; the status tabs narrow the list; the same
/// route answers JSON to a client that does not ask for a page.
#[renox::test]
async fn the_list_shows_the_demo_timesheets_in_words_and_narrows_by_status() {
    let app = demo().await;
    let html = page(&app, AMAL, "/timesheets").await;
    says(
        &html,
        &[
            "Timesheets",
            "Gulf Tower Facilities LLC",
            "Wasim Khan",
            "Ravi Kumar",
            "Joseph Mensah",
            "Draft",
            "Submitted",
            "Approved",
            "Billed",
            "Security guard",
            "56.00 h",
            "58.00 h",
            &day(0).format("%b %Y").to_string(),
            // The status tabs are the grid's filter now.
            "rx-grid__scroll",
        ],
    );
    assert!(!html.trim_start().starts_with('{'), "a page, not JSON");
    no_raw_words(&html);
    fits_a_phone(&html);

    let submitted = page(&app, AMAL, "/timesheets?status=submitted").await;
    let text = visible(&submitted);
    assert!(text.contains("Ravi Kumar") && text.contains("Bilal Ahmed"));
    assert!(!text.contains("Wasim Khan"), "a draft is not in Submitted");

    // The billed sheet is a worker of Al Quoz; narrowed to Gulf Tower it goes.
    let client = manpower_screens_support::client(&app, "Gulf Tower Facilities LLC").await;
    let all = visible(&html);
    let one = visible(&page(&app, AMAL, &format!("/timesheets?client={}", client.0)).await);
    assert!(all.matches("Billed").count() > one.matches("Billed").count());
    assert!(one.contains("Wasim Khan"));

    let user = person(&app, AMAL).await;
    app.acting_as(&user);
    let json = app.request().get("/timesheets").await;
    json.assert_status(200);
    assert!(json.text().trim_start().starts_with('['), "{}", json.text());
}

/// AC2 and the phone: a timesheet page shows the hours per day and category
/// with the totals, the facts and the history; the grid sits in its own
/// scrolling box.
#[renox::test]
async fn a_timesheet_shows_the_hours_per_day_and_category_with_totals() {
    let app = demo().await;
    let id = sheet_of(&app, "Ravi Kumar").await;
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Timesheet of Ravi Kumar",
            "Submitted",
            "Gulf Tower Facilities LLC",
            "Regular",
            "Overtime",
            "Public holiday",
            "8.00 h",
            "10.00 h",
            "58.00 h",
            "History",
            "The client",
            &day(-6).format("%a %d %b").to_string(),
            "rx-table-wrap",
            "rx-sheet",
        ],
    );
    no_raw_words(&html);
    fits_a_phone(&html);
}

/// AC3: *Submit* of a draft whose allocated hours are above the attended ones
/// is refused with the two hour figures; once the excess is gone it shows
/// *Submitted* and the history says who submitted it and when.
#[renox::test]
async fn submitting_a_draft_with_too_many_hours_is_refused_with_both_figures() {
    let app = demo().await;
    let id = sheet_of(&app, "Wasim Khan").await;
    let draft = page(&app, AMAL, &path(id)).await;
    says(&draft, &["Draft", "Submit", "id=\"submit-timesheet\""]);

    let first = TimesheetEntry::of_timesheet(app.db(), id).await.unwrap();
    let extra = TimesheetEntry::create(
        app.db(),
        TimesheetEntry {
            id: 0,
            overtime_minutes: 120,
            normal_minutes: 0,
            created_at: None,
            updated_at: None,
            ..first[1].clone()
        },
    )
    .await
    .unwrap();
    let refused = post(&app, AMAL, &format!("{}/submit", path(id)), &[]).await;
    refused.assert_status(422);
    let text = refused.text();
    assert!(
        text.contains("10 h allocated") && text.contains("8 h attended"),
        "{text}"
    );
    assert_eq!(stored(&app, id).await.status, TimesheetStatus::Open);

    let mut extra = extra;
    extra.delete(app.db()).await.unwrap();
    let sent = post(&app, AMAL, &format!("{}/submit", path(id)), &[]).await;
    assert!(sent.status.is_success() || sent.status.is_redirection());
    let html = page(&app, AMAL, &path(id)).await;
    says(&html, &["Submitted", "Amal Youssef"]);
    // The history row of the submit: when (today, with the time), what, who.
    let submitted = cells_of_row(&html, "Amal Youssef");
    assert!(
        submitted[0].starts_with(&words(day(0))),
        "when: {submitted:?}"
    );
    assert_eq!(submitted[1], "Submitted", "what: {submitted:?}");
    assert_eq!(submitted[2], "Amal Youssef", "who: {submitted:?}");
    assert_eq!(stored(&app, id).await.status, TimesheetStatus::Submitted);
}

/// AC4: a submitted sheet unanswered for 6 days shows *Escalated* with the date
/// and the days; after 10 days, with the clause of the contract, it shows
/// *Deemed approved* with the days of the clause.
#[renox::test]
async fn an_unanswered_sheet_is_escalated_and_then_deemed_approved_with_the_clause() {
    let app = demo().await;
    // Ravi's contract has the clause of 10 days; he submitted two days ago, so
    // the sheet is six days old four days from now.
    let id = sheet_of(&app, "Ravi Kumar").await;
    escalation::run(app.state(), day(4)).await.unwrap();
    let escalated = page(&app, AMAL, &path(id)).await;
    says(
        &escalated,
        &[
            "Escalated",
            "Escalated on",
            "6 days without answer",
            &stored(&app, id)
                .await
                .escalated_at
                .map(|at| {
                    wl_kernel::documents::dubai_date(at)
                        .format("%d %b")
                        .to_string()
                })
                .unwrap(),
        ],
    );
    no_raw_words(&escalated);
    let list = page(&app, AMAL, "/timesheets?status=escalated").await;
    assert!(visible(&list).contains("Ravi Kumar"));

    escalation::run(app.state(), day(8)).await.unwrap();
    let deemed = page(&app, AMAL, &path(id)).await;
    says(
        &deemed,
        &[
            "Deemed approved",
            "no answer in 10 days",
            "as the client contract says",
        ],
    );
    assert!(
        !deemed.contains("id=\"approve-timesheet\""),
        "nothing left to approve"
    );
    no_raw_words(&deemed);
}

/// AC5: *Reject* with an empty reason says *Write the reason*; with a reason the
/// sheet is *Rejected* and the page shows the reason.
#[renox::test]
async fn rejecting_needs_a_reason_and_the_page_shows_it() {
    let app = demo().await;
    let id = sheet_of(&app, "Ravi Kumar").await;
    let reject = format!("{}/reject", path(id));
    let refused = post(&app, AMAL, &reject, &[("reason", " ")]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Write the reason"),
        "{}",
        refused.text()
    );
    assert_eq!(stored(&app, id).await.status, TimesheetStatus::Submitted);

    post(
        &app,
        AMAL,
        &reject,
        &[("reason", "The overtime on the third day is not agreed")],
    )
    .await;
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Rejected",
            "Sent back",
            "The overtime on the third day is not agreed",
        ],
    );
    assert_eq!(stored(&app, id).await.status, TimesheetStatus::Open);
    no_raw_words(&html);
}

/// AC6: *Approve on behalf* without the signed sheet says to attach the
/// client's signed sheet; with it the sheet is *Approved* on behalf and the
/// signed sheet is linked.
#[renox::test]
async fn approving_on_behalf_needs_the_signed_sheet_and_links_it() {
    let app = demo().await;
    let id = sheet_of(&app, "Ravi Kumar").await;
    let form = format!("{}/approve-on-behalf", path(id));
    let before = page(&app, AMAL, &path(id)).await;
    says(
        &before,
        &["Approve on behalf", "Documents of this timesheet"],
    );

    let refused = post(&app, AMAL, &form, &[("approver_name", "Omar")]).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .replace("&#39;", "'")
            .contains("Attach the client's signed sheet"),
        "{}",
        refused.text()
    );
    let document = scalar(&app, "select min(id) from wl_documents_documents").await;
    let signed_on = day(0).to_string();
    let approved = post(
        &app,
        AMAL,
        &form,
        &[
            ("document_id", &document.to_string()),
            ("approver_name", "Omar Haddad"),
            ("signed_on", &signed_on),
        ],
    )
    .await;
    assert!(approved.status.is_success() || approved.status.is_redirection());
    let html = page(&app, AMAL, &path(id)).await;
    says(
        &html,
        &[
            "Approved",
            "Approved on behalf of the client",
            "Omar Haddad",
            &format!("/documents/{document}/file"),
        ],
    );
    assert_eq!(stored(&app, id).await.status, TimesheetStatus::Approved);
    no_raw_words(&html);

    // Nothing has used the hours: the admin, who holds the permission, can still
    // reopen them; the account manager is not offered the button.
    assert!(!html.contains("id=\"reopen-timesheet\""));
    let admin = page(&app, ADMIN, &path(id)).await;
    says(&admin, &["Reopen", "id=\"reopen-timesheet\""]);
    post(
        &app,
        ADMIN,
        &format!("{}/reopen", path(id)),
        &[("reason", "The client sent a corrected sheet")],
    )
    .await;
    let reopened = page(&app, ADMIN, &path(id)).await;
    says(
        &reopened,
        &["Draft", "Reopened", "The client sent a corrected sheet"],
    );
    assert_eq!(stored(&app, id).await.status, TimesheetStatus::Open);
}

/// AC7 and AC8: on a billed sheet the page says *Billed* with the invoice linked
/// and offers no submit, approve, reject or reopen; an adjustment opened by the
/// admin is refused to the admin who approves it (the refusal text), and the
/// account manager's approval on behalf succeeds.
#[renox::test]
async fn a_billed_sheet_is_read_only_and_its_adjustment_is_approved_by_someone_else() {
    let app = demo().await;
    let id = scalar(
        &app,
        "select id from wl_attendance_timesheets where status = 'locked' \
         and billed_by is not null",
    )
    .await;
    let billed = page(&app, AMAL, &path(id)).await;
    says(
        &billed,
        &[
            "Billed",
            "Billed on invoice",
            "Open the invoice",
            "/invoices/",
        ],
    );
    for gone in [
        "id=\"submit-timesheet\"",
        "id=\"approve-timesheet\"",
        "id=\"reject-timesheet\"",
        "id=\"reopen-timesheet\"",
    ] {
        assert!(
            !billed.contains(gone),
            "{gone} is not offered on a billed sheet"
        );
    }
    no_raw_words(&billed);

    // The admin opens the adjustment and submits it.
    let start = day(-40).to_string();
    let opened = post(
        &app,
        ADMIN,
        &format!("{}/adjustments", path(id)),
        &[
            ("reason", "Two hours were counted on a day off"),
            ("origin", "attendance_error"),
            ("affects", "billing"),
            ("entry_date", &start),
            ("entry_category", "normal"),
            ("entry_hours", "-2"),
        ],
    )
    .await;
    assert!(opened.status.is_success() || opened.status.is_redirection());
    let adjustment = scalar(
        &app,
        &format!("select max(id) from wl_attendance_timesheets where adjusts_id = {id}"),
    )
    .await;
    post(&app, ADMIN, &format!("{}/submit", path(adjustment)), &[]).await;
    assert_eq!(
        stored(&app, adjustment).await.status,
        TimesheetStatus::Submitted
    );

    let original = page(&app, AMAL, &path(id)).await;
    says(
        &original,
        &["Adjustments", "Two hours were counted on a day off"],
    );

    let as_admin = page(&app, ADMIN, &path(adjustment)).await;
    says(
        &as_admin,
        &[
            "Adjustment",
            "Open the timesheet it corrects",
            "You opened this adjustment: another person must approve it.",
        ],
    );
    let document = scalar(&app, "select min(id) from wl_documents_documents").await;
    let signed_on = day(0).to_string();
    let form = [
        ("document_id", document.to_string()),
        ("approver_name", "Omar Haddad".to_owned()),
        ("signed_on", signed_on),
    ];
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let own = post(
        &app,
        ADMIN,
        &format!("{}/approve-on-behalf", path(adjustment)),
        &form,
    )
    .await;
    own.assert_status(422);
    assert!(
        own.text().contains("cannot approve it"),
        "the creator is refused with the rule's words: {}",
        own.text()
    );

    let other = post(
        &app,
        AMAL,
        &format!("{}/approve-on-behalf", path(adjustment)),
        &form,
    )
    .await;
    assert!(other.status.is_success() || other.status.is_redirection());
    assert_eq!(
        stored(&app, adjustment).await.status,
        TimesheetStatus::Approved
    );
}

/// A person who holds attendance only for a site sees no timesheet of the
/// clients and cannot open one by its address; HR, who holds it everywhere,
/// sees them all.
#[renox::test]
async fn the_supervisor_sees_no_client_timesheet_and_hr_sees_them_all() {
    let app = demo().await;
    let id = sheet_of(&app, "Ravi Kumar").await;
    let list = page(&app, SUPERVISOR, "/timesheets").await;
    assert!(!visible(&list).contains("Ravi Kumar"));
    let denied = browse(&app, SUPERVISOR, &path(id)).await;
    denied.assert_status(403);
    let all = page(&app, HR, "/timesheets").await;
    assert!(visible(&all).contains("Ravi Kumar"));
    // The deployment page links its timesheets to the new page.
    let deployment = manpower_screens_support::deployment_of(&app, "Ravi Kumar").await;
    let tab = page(&app, AMAL, &format!("/manpower/deployments/{deployment}")).await;
    says(&tab, &[&format!("/timesheets/{id}")]);
}
