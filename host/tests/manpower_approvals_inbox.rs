//! BL-MPW-29 (#509), `docs/UX.md` §2.1: the approvals inbox in the demo
//! agency: `/approvals` lists what waits for the signed-in person in words
//! (the action, the record by its name, who asked, why) with *Approve*,
//! *Reject* and *Return for changes*; the requester sees *Waiting for the MD*
//! and no button; reject and return need a note; the six kinds of request of
//! the agency read in their own words; the dashboard links to the inbox and
//! shows no raw key. Each test opens the page as a role of the demo agency and
//! checks the words.

mod manpower_screens_support;
mod support;

use manpower_screens_support::{
    ADMIN, AMAL, HR, MD, SUPERVISOR, browse, contract, day, demo, employee, fits_a_phone,
    no_raw_words, page, post, says, scalar, visible,
};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::approvals::{ApprovalPolicy, awaiting::action_label};

/// Asserts that the answer is a success, showing what the page said if not.
fn done(response: renox::testing::TestResponse) {
    assert!(
        response.status.is_success() || response.status.is_redirection(),
        "{}: {}",
        response.status,
        visible(&response.text())
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    );
}

/// The pending request of `action`.
async fn request_of(app: &TestApp, action: &str) -> i64 {
    scalar(
        app,
        &format!(
            "select id from wl_approval_requests where status = 'pending' and action = '{action}'"
        ),
    )
    .await
}

/// Al Quoz's contract is on hold for its overdue invoice (the demo seed ran the
/// credit hold job); the account manager asks for the release: the MD must
/// decide.
async fn release_asked(app: &TestApp) -> i64 {
    let id = contract(app, "CC-DEMO-003").await;
    post(app, AMAL, &format!("/manpower/contracts/{id}/release"), &[])
        .await
        .assert_status(303);
    id
}

/// AC1 and AC2 and AC3: the MD's row reads *Release a client contract from a
/// credit hold*, the client, who asked and why, with the three buttons; the
/// account manager sees *Waiting for the MD* and no button; *Approve* makes the
/// row go and the contract active.
#[renox::test]
async fn the_md_decides_a_release_the_account_manager_only_waits_for() {
    let app = demo().await;
    let id = release_asked(&app).await;
    let request = request_of(&app, "manpower.release").await;

    let md = page(&app, MD, "/approvals").await;
    says(
        &md,
        &[
            "Approvals",
            "Release a client contract from a credit hold",
            "CC-DEMO-003, Al Quoz Logistics Park LLC",
            "Requested by Amal Youssef",
            "Reason",
            "Invoice overdue",
            "Approve",
            "Reject",
            "Return for changes",
            &format!("id=\"approve-{request}\""),
            &format!("id=\"reject-{request}\""),
            &format!("id=\"return-{request}\""),
            &format!("href=\"/manpower/contracts/{id}\""),
            "rx-card",
        ],
    );
    no_raw_words(&md);
    fits_a_phone(&md);

    let amal = page(&app, AMAL, "/approvals").await;
    says(
        &amal,
        &[
            "Waiting for the MD",
            "Release a client contract from a credit hold",
            "Nothing needs your action",
        ],
    );
    assert!(
        !amal.contains(&format!("id=\"approve-{request}\"")),
        "the requester has no button"
    );
    no_raw_words(&amal);
    // The requester cannot decide through the route either.
    post(&app, AMAL, &format!("/approvals/{request}/approve"), &[])
        .await
        .assert_status(403);

    post(&app, MD, &format!("/approvals/{request}/approve"), &[])
        .await
        .assert_ok();
    let after = page(&app, MD, "/approvals").await;
    assert!(!after.contains("Release a client contract from a credit hold"));
    says(&after, &["Nothing needs your action"]);
    says(
        &page(&app, MD, &format!("/manpower/contracts/{id}")).await,
        &["Active"],
    );
    assert!(
        !page(&app, AMAL, "/approvals")
            .await
            .contains("Waiting for the MD"),
        "the request is no longer open"
    );
}

/// AC3 and AC4: *Reject* and *Return for changes* without a note are refused in
/// words; with a note the return reaches the requester's inbox and the contract
/// page, and the requester asks again.
#[renox::test]
async fn reject_and_return_need_a_note_and_the_requester_sees_it() {
    let app = demo().await;
    let id = release_asked(&app).await;
    let request = request_of(&app, "manpower.release").await;
    for action in ["reject", "return"] {
        let refused = post(
            &app,
            MD,
            &format!("/approvals/{request}/{action}"),
            &[("comment", " ")],
        )
        .await;
        refused.assert_status(422);
        assert!(
            refused
                .text()
                .contains("A comment is required to reject or return."),
            "{action}: {}",
            refused.text()
        );
    }
    post(
        &app,
        MD,
        &format!("/approvals/{request}/return"),
        &[("comment", "Attach the payment plan first")],
    )
    .await
    .assert_ok();
    let amal = page(&app, AMAL, "/approvals").await;
    says(
        &amal,
        &[
            "Returned to you: Attach the payment plan first",
            "Release a client contract from a credit hold",
        ],
    );
    no_raw_words(&amal);
    says(
        &page(&app, AMAL, &format!("/manpower/contracts/{id}")).await,
        &["Attach the payment plan first"],
    );
    // He asks again; the MD sees a new row and rejects it with a note.
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{id}/release"),
        &[],
    )
    .await
    .assert_status(303);
    let again = request_of(&app, "manpower.release").await;
    assert_ne!(again, request, "a new request");
    says(
        &page(&app, MD, "/approvals").await,
        &["Release a client contract from a credit hold"],
    );
    post(
        &app,
        MD,
        &format!("/approvals/{again}/reject"),
        &[("comment", "Not before the invoice is paid")],
    )
    .await
    .assert_ok();
    assert!(
        !page(&app, MD, "/approvals")
            .await
            .contains("Release a client contract from a credit hold")
    );
}

/// AC5: a termination with a waiver, a death, a waiver of the pay account
/// cooling, a past-dated rate card and a write-off each read in their own
/// words with the name of their record, never an id or a key.
#[renox::test]
async fn each_kind_of_request_reads_in_its_own_words_with_its_subject_name() {
    let app = demo().await;
    // A termination inside the notice period, with a waiver.
    let tower = contract(&app, "CC-DEMO-001").await;
    let (notice, end) = (day(0).to_string(), day(10).to_string());
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{tower}/terminate"),
        &[
            ("initiated_by", "client"),
            ("reason", "The client closes the site"),
            ("notice_on", notice.as_str()),
            ("end_date", end.as_str()),
            ("waiver", "waiver-signed.pdf"),
        ],
    )
    .await
    .assert_status(303);
    // The death of a worker, asked by HR.
    let noel = employee(&app, "Kamal Hossain").await;
    let died = day(-1).to_string();
    done(
        post(
            &app,
            HR,
            &format!("/hr/employees/{noel}/exits"),
            &[
                ("transition", "record_death"),
                ("reason", "died in service"),
                ("death_certificate", "DC-9"),
                ("death_date", died.as_str()),
            ],
        )
        .await,
    );
    // The waiver of the cooling of Ravi's new pay account.
    let ravi = employee(&app, "Ravi Kumar").await;
    let account = scalar(
        &app,
        &format!("select id from wl_hr_pay_accounts where employee_id = {ravi}"),
    )
    .await;
    done(
        post(
            &app,
            ADMIN,
            &format!("/hr/pay-accounts/{account}/waive"),
            &[("reason", "He is paid tomorrow")],
        )
        .await,
    );
    // A past-dated rate card the client accepted.
    let card = scalar(
        &app,
        &format!(
            "select id from wl_invoicing_rate_cards where source_type = 'manpower.client_contract' \
             and source_id = {tower}"
        ),
    )
    .await;
    let from = day(-1).to_string();
    let made = post(
        &app,
        AMAL,
        &format!("/rate-cards/{card}/versions"),
        &[("effective_from", &from)],
    )
    .await;
    made.assert_status(303);
    let version = manpower_screens_support::landed(&made);
    post(
        &app,
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
    post(
        &app,
        AMAL,
        &format!("/rate-card-versions/{version}/accept"),
        &[("document", "past-dated-signed.pdf")],
    )
    .await
    .assert_status(303);
    // A write-off of the overdue invoice, asked by the accountant.
    let invoice = scalar(
        &app,
        "select id from wl_invoicing_invoices where status in ('overdue', 'issued') order by id limit 1",
    )
    .await;
    post(
        &app,
        "accountant@desertstar.example",
        &format!("/invoices/{invoice}/write-off"),
        &[("reason_code", "insolvent"), ("reason_text", "")],
    )
    .await
    .assert_ok();

    let number = manpower_screens_support::text(
        &app,
        &format!("select number from wl_invoicing_invoices where id = {invoice}"),
    )
    .await;
    assert!(number.starts_with("INV"), "the invoice number: {number}");

    // The MD decides the termination, the death and the rate card; the waiver
    // of the cooling goes to the owner role of this deployment, which HR is
    // configured with (`md_role`), and the write-off to whoever the policy
    // names: the inbox of each says what waits for them.
    let md = page(&app, MD, "/approvals").await;
    let admin = page(&app, ADMIN, "/approvals").await;
    let text = format!("{} {}", visible(&md), visible(&admin));
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for words in [
        "Terminate a client contract",
        "CC-DEMO-001, Gulf Tower Facilities LLC",
        "The client closes the site",
        "non-standard exit",
        "Kamal Hossain",
        "died in service",
        "Waive the pay account cooling period",
        "Ravi Kumar",
        "He is paid tomorrow",
        "Approve a past-dated rate card",
        "Rate card of Gulf Tower Facilities LLC",
        "Write off an invoice",
        number.as_str(),
    ] {
        assert!(
            text.contains(words),
            "the inboxes do not say `{words}`: {one_line}"
        );
    }
    no_raw_words(&md);
    no_raw_words(&admin);
    for raw in [
        " #",
        "hr.employee",
        "hr.pay_account",
        "invoicing.",
        "manpower.",
        "payments.",
    ] {
        assert!(
            !text.contains(raw),
            "the inbox shows `{raw}`: {}",
            one_line.find(raw).map_or("", |at| &one_line
                [at.saturating_sub(100).min(one_line.len())..(at + 100).min(one_line.len())])
        );
    }
}

/// AC7: the site supervisor has nothing to decide: the inbox says so.
#[renox::test]
async fn a_person_with_nothing_to_decide_is_told_so() {
    let app = demo().await;
    let html = page(&app, SUPERVISOR, "/approvals").await;
    says(&html, &["Nothing needs your action", "No open requests"]);
    no_raw_words(&html);
    // JSON for a client that does not ask for a page.
    let json = {
        let user = manpower_screens_support::person(&app, SUPERVISOR).await;
        app.acting_as(&user);
        app.request().get("/approvals").await
    };
    json.assert_status(200);
    assert!(json.text().trim_start().starts_with('{'), "{}", json.text());
}

/// AC6: the dashboard's *1 approval waiting for you* links to the inbox and no
/// raw key is on the page.
#[renox::test]
async fn the_dashboard_row_links_to_the_inbox_and_shows_no_raw_key() {
    let app = demo().await;
    release_asked(&app).await;
    let html = page(&app, MD, "/dashboard").await;
    says(
        &html,
        &[
            "1 approval waiting for you",
            "href=\"/approvals\"",
            "Release a client contract from a credit hold",
        ],
    );
    assert!(
        !html.contains("manpower.release"),
        "a raw key on the dashboard"
    );
    let nav = page(&app, MD, "/approvals").await;
    says(&nav, &["href=\"/approvals\""]);
}

/// Every action the demo agency has a policy for has words, and an unknown one
/// reads *Approval needed*, never its key.
#[renox::test]
async fn every_action_with_a_policy_has_words_and_an_unknown_one_says_approval_needed() {
    let app = demo().await;
    let lang = app.state().lang("en");
    let policies = ApprovalPolicy::query().get(app.db()).await.unwrap();
    assert!(policies.len() > 20);
    for policy in policies {
        let words = action_label(&lang, &policy.action);
        assert_ne!(words, "Approval needed", "`{}` has no words", policy.action);
        assert!(!words.contains('.'), "`{}` reads `{words}`", policy.action);
    }
    assert_eq!(action_label(&lang, "nobody.knows_this"), "Approval needed");
    browse(&app, MD, "/approvals").await.assert_status(200);
}
