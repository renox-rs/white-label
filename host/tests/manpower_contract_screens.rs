//! BL-MPW-22 (#502): the screens of a client contract in the demo agency: the
//! list with *New contract* and a link per row, the record page with its
//! actions and tabs, the sheets and their refusals in words, the credit hold,
//! its release by the MD, the early termination, the renewal, the deployments
//! tab, the sites, and the roles that may not act. Each test opens the page as
//! a role of the demo agency and checks the words on it.

mod manpower_screens_support;
mod support;

use manpower::ContractStatus;
use manpower_screens_support::{
    ADMIN, AMAL, MD, SUPERVISOR, accepted_rate_card, browse, client, contract, day, demo,
    draft_contract, no_raw_words, page, person, post, says, scalar, stored, words, worker_of,
};
use renox::prelude::*;

/// AC1: the account manager opens `/manpower/contracts`: *New contract* is
/// there and every row links to its page.
#[renox::test]
async fn the_list_offers_new_contract_and_links_each_row_to_its_page() {
    let app = demo().await;
    let html = page(&app, AMAL, "/manpower/contracts").await;
    says(&html, &["New contract"]);
    for reference in ["CC-DEMO-001", "CC-DEMO-002", "CC-DEMO-003"] {
        let id = contract(&app, reference).await;
        assert!(
            html.contains(&format!("/manpower/contracts/{id}\"")),
            "{reference} has no link to its page"
        );
    }
    no_raw_words(&html);
    // The JSON of the list is what it was.
    let user = person(&app, AMAL).await;
    app.acting_as(&user);
    let json = app.request().json().get("/manpower/contracts").await;
    json.assert_ok();
    assert!(json.text().contains("CC-DEMO-001"));
}

/// AC2: *New contract* with an end date before the start date keeps the form
/// and says why; a good one makes a draft and lands on its page.
#[renox::test]
async fn a_new_contract_with_the_end_before_the_start_is_refused_in_words() {
    let app = demo().await;
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    let (start, end) = (day(30).to_string(), day(10).to_string());
    let refused = post(
        &app,
        AMAL,
        "/manpower/contracts",
        &[
            ("client_party_id", &tower.0.to_string()),
            ("reference", "CC-NEW-0"),
            ("start_date", &start),
            ("end_date", &end),
        ],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("The end date must be after the start date"),
        "{}",
        refused.text()
    );
    // The refusal is carried as the page's error detail, which forms.js shows at
    // the top of the sheet that sent it: the sheet is not swapped or reloaded, so
    // the browser keeps what was typed (the answer is the error page, not the
    // form again).
    let body = refused.text();
    assert!(
        body.contains("data-error-detail=\"The end date must be after the start date"),
        "{body}"
    );
    assert_eq!(
        scalar(&app, "select count(*) from manpower_client_contracts").await,
        3,
        "nothing was made"
    );
    // The sheet that sends it is on the list page, with the fields to refill,
    // and posts to the same route.
    let html = page(&app, AMAL, "/manpower/contracts").await;
    says(
        &html,
        &[
            "new-contract",
            "First day",
            "Last day",
            "Notice days",
            "name=\"reference\"",
            "name=\"start_date\"",
            "name=\"end_date\"",
            "action=\"/manpower/contracts\"",
        ],
    );

    let id = draft_contract(&app, "CC-NEW-1").await;
    let draft = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &draft,
        &[
            "CC-NEW-1",
            "Draft",
            "Gulf Tower Facilities LLC",
            "Mark signed",
        ],
    );
    no_raw_words(&draft);
}

/// AC3: *Mark signed* without a document says to attach it first; with the
/// document and the day the contract is still a draft and the page shows when
/// it was signed and the document.
#[renox::test]
async fn mark_signed_needs_the_document_and_then_shows_the_signature() {
    let app = demo().await;
    let id = draft_contract(&app, "CC-NEW-2").await;
    let path = format!("/manpower/contracts/{id}/signed");
    let signed_on = day(0).to_string();
    let refused = post(&app, AMAL, &path, &[("signed_on", &signed_on)]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Attach the signed contract first"),
        "{}",
        refused.text()
    );
    post(
        &app,
        AMAL,
        &path,
        &[
            ("signed_on", &signed_on),
            ("document", "CC-NEW-2-signed.pdf"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(stored(&app, id).await.status, ContractStatus::Draft);
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &html,
        &[
            "Draft",
            &format!("Signed on {}", words(day(0))),
            "CC-NEW-2-signed.pdf",
        ],
    );
}

/// AC4: a signed draft with an accepted rate card version is activated and the
/// page says *Active*; without the version the button is disabled with the
/// reason and the route says *Accept a rate card version first*; a blocked
/// client is refused with the block's own words.
#[renox::test]
async fn activate_needs_the_accepted_rate_card_and_an_unblocked_client() {
    let app = demo().await;
    let id = draft_contract(&app, "CC-NEW-3").await;
    let signed_on = day(0).to_string();
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{id}/signed"),
        &[
            ("signed_on", &signed_on),
            ("document", "CC-NEW-3-signed.pdf"),
        ],
    )
    .await
    .assert_status(303);
    let path = format!("/manpower/contracts/{id}/activate");

    let refused = post(&app, AMAL, &path, &[]).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Accept a rate card version first"),
        "{}",
        refused.text()
    );
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &html,
        &[
            "Accept a rate card version first",
            "Before it can be activated",
        ],
    );
    assert_eq!(stored(&app, id).await.status, ContractStatus::Draft);

    // The client is blocked: the refusal is the block's.
    let tower = client(&app, "Gulf Tower Facilities LLC").await;
    support::block(&app, tower).await;
    accepted_rate_card(&app, id, day(0)).await;
    let blocked = post(&app, AMAL, &path, &[]).await;
    blocked.assert_status(422);
    assert!(
        blocked.text().contains("This party is on hold"),
        "{}",
        blocked.text()
    );
    post(
        &app,
        ADMIN,
        &format!("/parties/{}/unblock", tower.0),
        &[("note", "Cleared")],
    )
    .await
    .assert_ok();

    post(&app, AMAL, &path, &[]).await.assert_status(303);
    let active = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(&active, &["Active", "Put on hold", "Renew"]);
    assert!(
        !active.contains("Mark signed"),
        "an active contract is not signed again"
    );
}

/// AC5: holding an active contract for a bounced cheque shows *On hold*, the
/// reason, who held it and the state before; the same reason again is refused.
#[renox::test]
async fn a_hold_shows_its_reason_its_author_and_the_state_before() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let path = format!("/manpower/contracts/{id}/hold");
    post(
        &app,
        AMAL,
        &path,
        &[("reason", "bounced_cheque"), ("text", "CHQ-9")],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &html,
        &[
            "On hold",
            "Credit hold",
            "Bounced cheque CHQ-9",
            "Amal Youssef",
            "State before the hold",
            "Release",
        ],
    );
    no_raw_words(&html);
    let again = post(
        &app,
        AMAL,
        &path,
        &[("reason", "bounced_cheque"), ("text", "CHQ-9")],
    )
    .await;
    again.assert_status(422);
    assert!(
        again.text().contains("already on hold for that reason"),
        "{}",
        again.text()
    );
    // Another reason is added to the hold.
    post(
        &app,
        AMAL,
        &path,
        &[("reason", "other"), ("text", "Client in liquidation")],
    )
    .await
    .assert_status(303);
    let more = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(&more, &["Bounced cheque CHQ-9", "Client in liquidation"]);
}

/// AC6: a held contract with an overdue invoice: *Release* says *Waiting for
/// the MD*; the requester cannot approve (403 with the refusal text); the MD
/// opens the page, approves, and the contract is active again.
#[renox::test]
async fn releasing_a_hold_with_an_overdue_invoice_waits_for_the_md() {
    let app = demo().await;
    // The demo seed has already run the credit hold job on the overdue invoice.
    let id = contract(&app, "CC-DEMO-003").await;
    post(
        &app,
        AMAL,
        &format!("/manpower/contracts/{id}/release"),
        &[],
    )
    .await
    .assert_status(303);
    let waiting = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &waiting,
        &[
            "Waiting for the MD",
            "On hold",
            "Release a client contract from a credit hold",
        ],
    );
    assert!(
        !waiting.contains("id=\"release-contract\""),
        "the release button is gone while the MD decides"
    );
    no_raw_words(&waiting);

    let request = scalar(
        &app,
        "select id from wl_approval_requests where status = 'pending' and action = 'manpower.release'",
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

    let for_md = page(&app, MD, &format!("/manpower/contracts/{id}")).await;
    says(
        &for_md,
        &[
            "Approve",
            "Reject",
            "Release a client contract from a credit hold",
        ],
    );
    let approved = post(&app, MD, &approve, &[]).await;
    assert!(approved.status.is_success() || approved.status.is_redirection());
    let html = page(&app, MD, &format!("/manpower/contracts/{id}")).await;
    says(&html, &["Active", "Approved"]);
    assert!(
        !html.contains("This contract is on credit hold"),
        "the hold is gone"
    );
    assert_eq!(stored(&app, id).await.status, ContractStatus::Active);
}

/// AC7: *Terminate* with an end date inside the notice period and no waiver
/// says so; with the waiver it asks the MD, and the page says *Waiting for the
/// MD*.
#[renox::test]
async fn an_early_termination_inside_the_notice_needs_a_waiver_and_the_md() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let path = format!("/manpower/contracts/{id}/terminate");
    let (notice, end) = (day(0).to_string(), day(10).to_string());
    let form = |waiver: &'static str| {
        vec![
            ("initiated_by", "client"),
            ("reason", "The client closes the site"),
            ("notice_on", notice.as_str()),
            ("end_date", end.as_str()),
            ("waiver", waiver),
        ]
    };
    let refused = post(&app, AMAL, &path, &form("")).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("inside the notice period"),
        "{}",
        refused.text()
    );
    assert_eq!(stored(&app, id).await.status, ContractStatus::Active);

    post(&app, AMAL, &path, &form("waiver-signed.pdf"))
        .await
        .assert_status(303);
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &html,
        &["Waiting for the MD", "Terminate a client contract"],
    );
    assert_eq!(
        stored(&app, id).await.status,
        ContractStatus::Active,
        "nothing is cut until the MD approves"
    );
}

/// AC8: *Renew* with a later end date and rates from the day of a rate card
/// version: the page shows the new end date and lists the version in the *Rate
/// cards* tab; an end date that is not later is refused in words.
#[renox::test]
async fn a_renewal_shows_the_new_end_date_and_the_rate_card_version() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let path = format!("/manpower/contracts/{id}/renew");
    let signed_on = day(0).to_string();
    let current_end = stored(&app, id).await.end_date;
    let not_later = current_end.to_string();
    let refused = post(
        &app,
        AMAL,
        &path,
        &[
            ("end_date", &not_later),
            ("signed_on", &signed_on),
            ("document", "RENEWAL.pdf"),
        ],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("must be later than the current end date"),
        "{}",
        refused.text()
    );

    // The demo contract has a version waiting for the client, from day 30.
    let rates_from = day(30);
    let before = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &before,
        &["Rate cards", &words(rates_from), "Waiting for the client"],
    );
    let (new_end, from) = (day(400).to_string(), rates_from.to_string());
    let form = [
        ("end_date", new_end.as_str()),
        ("signed_on", signed_on.as_str()),
        ("document", "RENEWAL.pdf"),
        ("rates_from", from.as_str()),
    ];
    // Renewing with changed rates needs the version accepted first.
    post(&app, AMAL, &path, &form).await.assert_status(422);
    let version = scalar(
        &app,
        "select v.id from wl_invoicing_rate_card_versions v where v.status = 'pending'",
    )
    .await;
    post(
        &app,
        ADMIN,
        &format!("/rate-card-versions/{version}/accept"),
        &[("document", "signed-acceptance.pdf")],
    )
    .await
    .assert_status(303);
    post(&app, AMAL, &path, &form).await.assert_status(303);
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(
        &html,
        &[
            &words(day(400)),
            "Renewed on",
            "Accepted",
            &words(rates_from),
        ],
    );
}

/// AC9: the *Deployments* tab lists the contract's deployments with worker,
/// role, site, dates and status in words, each linking to its page.
#[renox::test]
async fn the_deployments_tab_lists_the_workers_with_links() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    let deployment = scalar(
        &app,
        &format!(
            "select min(id) from manpower_deployments where client_contract_id = {id} and status = 'active'"
        ),
    )
    .await;
    let worker = worker_of(&app, deployment).await;
    let start = manpower::Deployment::find_or_404(app.db(), deployment)
        .await
        .unwrap()
        .start_date;
    says(
        &html,
        &[
            &format!("/manpower/deployments/{deployment}\""),
            "Security guard",
            "Tower A, lobby and security desk",
            "Active",
            &worker,
            &words(start),
        ],
    );
    no_raw_words(&html);
}

/// AC10: the site supervisor has no contract screen, and a direct post of a
/// hold answers 403; a person who may only view a contract sees no action.
#[renox::test]
async fn people_who_may_not_manage_a_contract_see_no_button_and_get_403() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    browse(&app, SUPERVISOR, &format!("/manpower/contracts/{id}"))
        .await
        .assert_forbidden();
    browse(&app, SUPERVISOR, "/manpower/contracts")
        .await
        .assert_forbidden();
    post(
        &app,
        SUPERVISOR,
        &format!("/manpower/contracts/{id}/hold"),
        &[("reason", "overdue")],
    )
    .await
    .assert_forbidden();
    assert_eq!(stored(&app, id).await.status, ContractStatus::Active);

    // Someone who holds `contracts.view` only.
    let viewer = renox::auth::User::register(app.db(), "Vera", "vera@test.example", "secret123")
        .await
        .unwrap();
    renox::auth::permissions::define_role(
        app.db(),
        "contract_viewer",
        &[manpower::permissions::CONTRACTS_VIEW],
    )
    .await
    .unwrap();
    let tower = stored(&app, id).await.scope();
    viewer
        .assign_role_in(app.db(), "contract_viewer", &tower)
        .await
        .unwrap();
    app.acting_as(&viewer);
    let seen = app
        .request()
        .header("accept", "text/html")
        .get(&format!("/manpower/contracts/{id}"))
        .await;
    seen.assert_ok();
    let html = seen.text();
    says(&html, &["CC-DEMO-001", "Active"]);
    for action in [
        "Put on hold",
        "Renew",
        "Terminate early",
        "Timesheet terms",
        "New site",
    ] {
        assert!(!html.contains(action), "a viewer is offered `{action}`");
    }
}

/// AC11: the account manager adds the site *Marina Tower* with a required
/// licence for Gulf Tower and it appears in the client's list; renaming it
/// keeps the deployments linked; a name the client already has is refused.
#[renox::test]
async fn a_site_is_added_renamed_and_never_duplicated() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let tower = client(&app, "Gulf Tower Facilities LLC")
        .await
        .0
        .to_string();
    let page_path = format!("/manpower/contracts/{id}");
    post(
        &app,
        AMAL,
        "/manpower/sites",
        &[
            ("client_party_id", &tower),
            ("name", "Marina Tower"),
            ("address", "Dubai Marina"),
            ("required_licences", "SIRA licence"),
        ],
    )
    .await
    .assert_status(303);
    let html = page(&app, AMAL, &page_path).await;
    says(
        &html,
        &["Marina Tower", "Dubai Marina", "SIRA licence", "New site"],
    );

    let duplicate = post(
        &app,
        AMAL,
        "/manpower/sites",
        &[("client_party_id", &tower), ("name", "marina tower")],
    )
    .await;
    duplicate.assert_status(422);
    assert!(
        duplicate
            .text()
            .contains("already has a site with that name"),
        "{}",
        duplicate.text()
    );

    // Renaming a site with a deployment keeps the deployment on it.
    let site = scalar(
        &app,
        "select site_id from manpower_deployments where status = 'active' and client_party_id = (select client_party_id from manpower_client_contracts where reference = 'CC-DEMO-001') limit 1",
    )
    .await;
    post(
        &app,
        AMAL,
        &format!("/manpower/sites/{site}"),
        &[
            ("name", "Tower A, main lobby"),
            ("address", "Sheikh Zayed Road"),
        ],
    )
    .await
    .assert_status(303);
    let after = page(&app, AMAL, &page_path).await;
    says(&after, &["Tower A, main lobby"]);
    assert!(
        scalar(
            &app,
            &format!("select count(*) from manpower_deployments where site_id = {site}")
        )
        .await
            >= 1,
        "the deployments stay on the renamed site"
    );
}

/// AC12: on a phone no table widens the page: each scrolls inside its own box
/// (the kit's table wrapper), the tabs are the kit's own scrolling bar, and the
/// forms are sheets that fit the screen.
#[renox::test]
async fn the_contract_page_and_its_forms_are_built_to_fit_a_phone() {
    let app = demo().await;
    let id = contract(&app, "CC-DEMO-001").await;
    let html = page(&app, AMAL, &format!("/manpower/contracts/{id}")).await;
    says(&html, &["rx-segmented", "rx-table-wrap", "rx-sheet"]);
    assert_eq!(
        html.matches("<table").count(),
        html.matches("rx-table-wrap").count(),
        "every table scrolls inside its own box, not the page"
    );
    let list = page(&app, AMAL, "/manpower/contracts").await;
    says(&list, &["rx-table-wrap", "rx-sheet"]);
    assert_eq!(
        list.matches("<table").count(),
        list.matches("rx-table-wrap").count()
    );
}

/// AC1 (the picker of *New contract*): it offers the client companies only: not
/// a worker (a person) and not a supplier of the agency, however they are named.
#[renox::test]
async fn the_party_picker_of_a_new_contract_lists_clients_only() {
    use wl_kernel::PartyKind;
    use wl_parties::{CompanyDetails, NewParty, PartyRole, create};
    let app = demo().await;
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Gulf Uniform Supplies LLC".into(),
            company: Some(CompanyDetails::default()),
            roles: vec![PartyRole {
                role_key: "supplier".into(),
                ..PartyRole::default()
            }],
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let found = page(&app, AMAL, "/manpower/parties/options?q=gulf").await;
    assert!(found.contains("Gulf Tower Facilities LLC"), "{found}");
    assert!(
        !found.contains("Gulf Uniform Supplies"),
        "a supplier: {found}"
    );
    let everyone = page(&app, AMAL, "/manpower/parties/options?q=").await;
    for client in ["Gulf Tower", "Marina Bay", "Al Quoz"] {
        assert!(everyone.contains(client), "{client}: {everyone}");
    }
    for not_a_client in ["Wasim Khan", "Ravi Kumar", "Carl Whitfield", "Uniform"] {
        assert!(
            !everyone.contains(not_a_client),
            "{not_a_client}: {everyone}"
        );
    }
}
