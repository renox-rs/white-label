//! BL-MPW-26 (#506), `MPW.ACL.04`, `MPW.ACL.16`: the rate card screens in the
//! demo agency: the card of a contract with its versions, a new version with
//! its lines, the client's acceptance with the signed document, the MD's
//! approval of a version dated in the past, the lines of an accepted version
//! that are never edited, the price preview, and the supervisor who sees no
//! rates. Each test opens the page as a role of the demo agency, posts the form
//! of its sheet and checks the words.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    AMAL, MD, SUPERVISOR, browse, contract, day, demo, draft_contract, fits_a_phone, landed,
    no_raw_words, page, post, says, scalar, words,
};
use renox::testing::TestApp;

/// The id of the rate card of contract `id`.
async fn card_of(app: &TestApp, id: i64) -> i64 {
    scalar(
        app,
        &format!(
            "select id from wl_invoicing_rate_cards where source_type = 'manpower.client_contract' \
             and source_id = {id}"
        ),
    )
    .await
}

/// The id of version `number` of card `card`.
async fn version_of(app: &TestApp, card: i64, number: i64) -> i64 {
    scalar(
        app,
        &format!(
            "select id from wl_invoicing_rate_card_versions where rate_card_id = {card} \
             and version = {number}"
        ),
    )
    .await
}

/// A new version of `card` from `from`, made through the sheet of the card by
/// the account manager; the id of the version.
async fn new_version(app: &TestApp, card: i64, from: renox::chrono::NaiveDate) -> i64 {
    let from = from.to_string();
    let made = post(
        app,
        AMAL,
        &format!("/rate-cards/{card}/versions"),
        &[("effective_from", &from)],
    )
    .await;
    made.assert_status(303);
    landed(&made)
}

/// The line *Security guard, day, AED 28.00 per hour, overtime 1.25* added to
/// `version` through its sheet.
async fn guard_line(app: &TestApp, version: i64) {
    post(
        app,
        AMAL,
        &format!("/rate-card-versions/{version}/lines"),
        &[
            ("role", "Security guard"),
            ("shift_type", "day"),
            ("unit", "hour"),
            ("rate", "28.00"),
            ("ot_multiplier", "1.25"),
        ],
    )
    .await
    .assert_status(303);
}

/// AC1: the contract's *Rate cards* tab links to the card, which lists the
/// versions with the effective date, the status in words and the acceptance,
/// as HTML; the JSON of the route still answers JSON.
#[renox::test]
async fn the_card_lists_the_versions_with_their_status_in_words() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let tab = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &tab,
        &["Open the rate card", &format!("/rate-cards/{card}")],
    );

    let html = page(&app, AMAL, &format!("/rate-cards/{card}")).await;
    says(
        &html,
        &[
            "Rate card",
            "v1",
            "v2",
            "In force",
            "Pending",
            "rate-card-v1-signed.pdf",
            "New version",
            &words(day(30)),
        ],
    );
    no_raw_words(&html);
    fits_a_phone(&html);
    assert!(html.contains("rx-sheet"), "the sheet of a new version");

    let user = manpower_screens_support::person(&app, AMAL).await;
    app.acting_as(&user);
    let json = app.request().get(&format!("/rate-cards/{card}")).await;
    json.assert_status(200);
    assert!(json.text().trim_start().starts_with('{'), "{}", json.text());
}

/// AC2: *New version* with a line makes a *Pending* version that prices nothing;
/// a second version with the same date is refused in words.
#[renox::test]
async fn a_new_version_is_pending_and_prices_nothing_and_the_same_date_is_refused() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let from = day(90);
    let version = new_version(&app, card, from).await;
    guard_line(&app, version).await;
    let html = page(&app, AMAL, &format!("/rate-card-versions/{version}")).await;
    says(
        &html,
        &[
            "Pending",
            "Security guard",
            "AED 28.00",
            "per hour",
            "1.25x",
            &words(from),
            "prices nothing until the client accepts",
        ],
    );
    no_raw_words(&html);
    // Priced on a day after its date, the version in force is still v1.
    let after = day(100).to_string();
    let preview = page(
        &app,
        AMAL,
        &format!("/rate-card-versions/{version}?line=security_guard%7Cday&date={after}&regular=1"),
    )
    .await;
    says(&preview, &["by version 1", "AED 28.00"]);

    let again = post(
        &app,
        AMAL,
        &format!("/rate-cards/{card}/versions"),
        &[("effective_from", &from.to_string())],
    )
    .await;
    again.assert_status(422);
    assert!(
        again.text().contains("already takes effect on that date"),
        "{}",
        again.text()
    );
}

/// AC3 and AC5: *Accept* without the document says to attach the client's
/// signed acceptance; with it the version shows *Accepted* with the document,
/// and from then on no line is added or edited (the buttons are gone and a
/// direct post is refused).
#[renox::test]
async fn accepting_needs_the_signed_document_and_then_the_lines_are_read_only() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let pending = version_of(&app, card, 2).await;
    let path = format!("/rate-card-versions/{pending}");

    let before = page(&app, AMAL, &path).await;
    says(&before, &["Accept for the client", "Add line", "Edit"]);

    let refused = post(&app, AMAL, &format!("{path}/accept"), &[]).await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .replace("&#39;", "'")
            .contains("Attach the client's signed acceptance"),
        "{}",
        refused.text()
    );
    post(
        &app,
        AMAL,
        &format!("{path}/accept"),
        &[("document", "rate-card-v2-signed.pdf")],
    )
    .await
    .assert_status(303);

    let html = page(&app, AMAL, &path).await;
    says(
        &html,
        &[
            "Accepted",
            "Accepted on",
            &words(day(0)),
            "Accepted by",
            "Amal Youssef",
            "rate-card-v2-signed.pdf",
            "never edited",
            "Security guard",
        ],
    );
    for gone in ["Add line", "Accept for the client", "id=\"add-rate-line\""] {
        assert!(!html.contains(gone), "{gone} is gone");
    }
    assert!(!html.contains("edit-rate-line-"), "no edit sheet");
    no_raw_words(&html);

    let line = scalar(
        &app,
        &format!("select min(id) from wl_invoicing_rate_card_lines where version_id = {pending}"),
    )
    .await;
    let edit = post(
        &app,
        AMAL,
        &format!("/rate-card-lines/{line}"),
        &[
            ("role", "security_guard"),
            ("shift_type", "day"),
            ("rate", "30"),
        ],
    )
    .await;
    edit.assert_status(422);
    assert!(edit.text().contains("new version"), "{}", edit.text());
    let add = post(
        &app,
        AMAL,
        &format!("{path}/lines"),
        &[("role", "Cook"), ("shift_type", "day"), ("rate", "20")],
    )
    .await;
    add.assert_status(422);
}

/// A version with no lines cannot be accepted: the button is disabled and says
/// to add a line, and a direct post says the same.
#[renox::test]
async fn a_version_without_lines_cannot_be_accepted() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let version = new_version(&app, card, day(120)).await;
    let html = page(&app, AMAL, &format!("/rate-card-versions/{version}")).await;
    says(
        &html,
        &[
            "Add at least one line",
            "No lines yet",
            "Accept for the client",
        ],
    );
    let refused = post(
        &app,
        AMAL,
        &format!("/rate-card-versions/{version}/accept"),
        &[("document", "x.pdf")],
    )
    .await;
    refused.assert_status(422);
    assert!(refused.text().contains("Add at least one line"));
}

/// AC4: a version dated yesterday that the client accepted *waits for the MD*;
/// the account manager who recorded the acceptance cannot approve it (403 with
/// the refusal text), the MD opens the page, approves, and it is *In force*.
#[renox::test]
async fn a_past_dated_version_waits_for_the_md_and_is_in_force_once_approved() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let version = new_version(&app, card, day(-1)).await;
    guard_line(&app, version).await;
    let path = format!("/rate-card-versions/{version}");
    post(
        &app,
        AMAL,
        &format!("{path}/accept"),
        &[("document", "past-dated-signed.pdf")],
    )
    .await
    .assert_status(303);

    let waiting = page(&app, AMAL, &path).await;
    says(
        &waiting,
        &[
            "Waiting for the MD",
            "Dated in the past",
            "past-dated-signed.pdf",
        ],
    );
    no_raw_words(&waiting);
    assert!(
        !waiting.contains("id=\"approve-rate-"),
        "the requester has no approve"
    );

    let request = scalar(
        &app,
        "select id from wl_approval_requests where status = 'pending' \
         and action = 'invoicing.rate_card_past_dated'",
    )
    .await;
    let approve = format!("/approvals/{request}/approve");
    let own = post(&app, AMAL, &approve, &[]).await;
    own.assert_status(403);
    assert!(
        own.text()
            .contains("You do not have permission for this action"),
        "{}",
        own.text()
    );

    let for_md = page(&app, MD, &path).await;
    says(
        &for_md,
        &["Approve", "Do not approve", "Waiting for the MD"],
    );
    let approved = post(&app, MD, &approve, &[]).await;
    assert!(approved.status.is_success() || approved.status.is_redirection());
    let html = page(&app, MD, &path).await;
    says(&html, &["In force", "Approved by"]);
    assert!(!html.contains("Waiting for the MD"));
    let card_html = page(&app, AMAL, &format!("/rate-cards/{card}")).await;
    says(&card_html, &["Superseded", "In force"]);
}

/// AC6: the preview of 10 regular hours and 2 overtime hours of a security
/// guard shows each line and the total in AED by the version in force on the
/// day; a day before the first accepted version says no rate card was in force.
#[renox::test]
async fn the_preview_prices_the_hours_by_the_version_in_force_on_the_day() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let version = version_of(&app, card, 1).await;
    let today = day(0).to_string();
    let html = page(
        &app,
        AMAL,
        &format!(
            "/rate-card-versions/{version}?line=security_guard%7Cday&date={today}&regular=10&overtime=2"
        ),
    )
    .await;
    says(
        &html,
        &[
            "Regular",
            "Overtime",
            "10.00 h",
            "2.00 h",
            "AED 280.00",
            "AED 70.00",
            "AED 350.00",
            "Total",
            "by version 1",
        ],
    );
    no_raw_words(&html);

    let early = day(-5000).to_string();
    let none = page(
        &app,
        AMAL,
        &format!("/rate-card-versions/{version}?line=security_guard%7Cday&date={early}&regular=8"),
    )
    .await;
    says(&none, &["No rate card in force on that date"]);
    assert!(!none.contains("AED 224.00"));
}

/// AC7: the supervisor holds no rate card permission: both pages answer 403
/// with the refusal text and show no rates, and the contract page of a person
/// without it has no link to the card.
#[renox::test]
async fn the_supervisor_sees_no_rates() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let card = card_of(&app, id).await;
    let version = version_of(&app, card, 1).await;
    for path in [
        format!("/rate-cards/{card}"),
        format!("/rate-card-versions/{version}"),
    ] {
        let denied = browse(&app, SUPERVISOR, &path).await;
        denied.assert_status(403);
        let text = denied.text();
        assert!(
            text.contains("You do not have permission for this action"),
            "{text}"
        );
        assert!(!text.contains("AED 28.00"), "no rates for the supervisor");
    }
    let direct = post(
        &app,
        SUPERVISOR,
        &format!("/rate-card-versions/{version}/accept"),
        &[("document", "x.pdf")],
    )
    .await;
    direct.assert_status(403);
}

/// A contract with no rate card offers *Start the rate card*, which makes the
/// empty card and lands on it with the invitation to add the first version.
#[renox::test]
async fn a_contract_without_a_card_starts_it_from_the_tab() {
    let app = demo().await;
    let id = draft_contract(&app, "CC-NEW-RATE").await;
    let tab = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(&tab, &["Start the rate card"]);
    let tower = manpower_screens_support::client(&app, "Gulf Tower Facilities LLC").await;
    let started = post(
        &app,
        AMAL,
        "/rate-cards",
        &[
            ("party", &tower.0.to_string()),
            ("source", &format!("manpower.client_contract:{id}")),
        ],
    )
    .await;
    started.assert_status(303);
    let html = page(&app, AMAL, &format!("/rate-cards/{}", landed(&started))).await;
    says(&html, &["No versions yet", "New version"]);
    let tab = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    assert!(!tab.contains("Start the rate card"));
    says(&tab, &["Open the rate card"]);
}
