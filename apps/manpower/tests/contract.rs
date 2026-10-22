//! Base 09 (09 Manpower §7.1 *Client contract*, §3.2): the contract is made for
//! a client company, signed, and `activate`d once the signed contract and a
//! rate card version accepted by the client are in place; `cancel` ends a
//! draft that never came into force. The account manager acts only in their
//! client companies. `hold`, `release`, `terminate`, `renew` and `end` (built later) come
//! with BL-MPW-01, -03, -04, -05 and -06.

mod support;

use manpower::{ClientContract, ContractStatus};
use renox::prelude::*;
use wl_kernel::Subject;

use support::{FOODS, TOWER, boot, date};

/// A draft contract of `client` made through the route, as `by`.
async fn made(t: &support::Booted, by: &User, client: i64, reference: &str) -> ClientContract {
    t.post(
        by,
        "/manpower/contracts",
        &[
            ("reference", reference),
            ("client_party_id", &client.to_string()),
            ("start_date", "2027-01-01"),
            ("end_date", "2027-12-31"),
            ("cover_required", "1"),
        ],
    )
    .await
    .assert_status(303);
    ClientContract::query()
        .where_(ClientContract::REFERENCE.eq(reference))
        .first(t.db())
        .await
        .unwrap()
        .expect("the contract was made")
}

async fn sign(t: &support::Booted, by: &User, contract: &ClientContract) {
    t.post(
        by,
        &format!("/manpower/contracts/{}/signed", contract.id),
        &[("signed_on", "2027-01-05"), ("document", "CC-SIGNED.pdf")],
    )
    .await
    .assert_status(303);
}

/// Base 09: the account manager makes a contract with its terms; it starts in
/// `draft`, with the manager as the account manager and no deemed approval.
#[renox::test]
async fn a_contract_is_made_in_draft_with_its_terms() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = made(&t, &amal, TOWER.0, "CC-TOWER-1").await;
    assert_eq!(contract.status, ContractStatus::Draft);
    assert_eq!(contract.client_party_id, TOWER.0);
    assert_eq!(contract.start_date, date(2027, 1, 1));
    assert_eq!(contract.end_date, date(2027, 12, 31));
    assert_eq!(contract.payment_terms_days, 30);
    assert_eq!(contract.notice_period_days, 30);
    assert!(contract.cover_required);
    assert_eq!(contract.deemed_approval_days, None);
    assert_eq!(contract.account_manager_user_id, Some(amal.id));
}

/// A contract needs a reference, an end date after the start, positive terms
/// and a known client.
#[renox::test]
async fn a_contract_without_a_reference_or_with_bad_dates_is_refused() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let form = |reference: &'static str, client: &'static str, end: &'static str| {
        vec![
            ("reference", reference),
            ("client_party_id", client),
            ("start_date", "2027-01-01"),
            ("end_date", end),
        ]
    };
    t.post(&amal, "/manpower/contracts", &form("", "31", "2027-12-31"))
        .await
        .assert_status(422);
    t.post(&amal, "/manpower/contracts", &form("X", "31", "2027-01-01"))
        .await
        .assert_status(422);
    // A client that is not a party; the account manager has no right there.
    t.post(&amal, "/manpower/contracts", &form("X", "99", "2027-12-31"))
        .await
        .assert_forbidden();
    let mut terms = form("X", "31", "2027-12-31");
    terms.push(("payment_terms_days", "0"));
    t.post(&amal, "/manpower/contracts", &terms)
        .await
        .assert_status(422);
    assert!(
        ClientContract::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
}

/// Base 09: `activate` of a signed contract with an accepted rate card version
/// in force on its start date makes it `active`, records the day and emits
/// `ContractActivated`.
#[renox::test]
async fn a_signed_contract_with_an_accepted_rate_card_is_activated() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    t.on(date(2027, 1, 10), async {
        let contract = made(&t, &amal, TOWER.0, "CC-TOWER-1").await;
        sign(&t, &amal, &contract).await;
        t.cards
            .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
        t.post(
            &amal,
            &format!("/manpower/contracts/{}/activate", contract.id),
            &[],
        )
        .await
        .assert_status(303);
        let active = t.reload_contract(contract.id).await;
        assert_eq!(active.status, ContractStatus::Active);
        assert_eq!(active.activated_on, Some(date(2027, 1, 10)));
        assert_eq!(active.signed_on, Some(date(2027, 1, 5)));
        assert_eq!(active.signed_document.as_deref(), Some("CC-SIGNED.pdf"));
        let events = t.activated.all();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].contract,
            Subject::new("manpower.client_contract", contract.id)
        );
        assert_eq!(events[0].client, TOWER);
        assert_eq!(events[0].end_date, date(2027, 12, 31));
    })
    .await;
}

/// Base 09: an unsigned contract with no accepted rate card is refused with
/// every failing check in one answer, and stays a draft.
#[renox::test]
async fn activation_lists_every_missing_check_and_changes_nothing() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    t.on(date(2027, 1, 10), async {
        let contract = made(&t, &amal, TOWER.0, "CC-TOWER-1").await;
        let response = t
            .post(
                &amal,
                &format!("/manpower/contracts/{}/activate", contract.id),
                &[],
            )
            .await;
        response.assert_status(422);
        let body = response.text();
        assert!(body.contains("has not signed the contract"), "{body}");
        assert!(body.contains("rate card version accepted"), "{body}");
        assert_eq!(
            t.reload_contract(contract.id).await.status,
            ContractStatus::Draft
        );
        assert!(t.activated.all().is_empty());
    })
    .await;
}

/// Base 09: a rate card version that is accepted but takes effect after the
/// start date of the contract does not count (a pending or later version
/// prices nothing before its day); the signed contract alone is not enough.
#[renox::test]
async fn a_rate_card_in_force_only_after_the_start_date_does_not_count() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    t.on(date(2027, 1, 10), async {
        let contract = made(&t, &amal, TOWER.0, "CC-TOWER-1").await;
        sign(&t, &amal, &contract).await;
        t.cards
            .accept(&contract.subject(), date(2027, 3, 1), &[("guard", "day")]);
        t.post(
            &amal,
            &format!("/manpower/contracts/{}/activate", contract.id),
            &[],
        )
        .await
        .assert_status(422);
        assert_eq!(
            t.reload_contract(contract.id).await.status,
            ContractStatus::Draft
        );
    })
    .await;
}

/// Base 09: a contract whose end date has passed cannot come into force.
#[renox::test]
async fn a_contract_that_is_over_is_not_activated() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = made(&t, &amal, TOWER.0, "CC-TOWER-1").await;
    sign_and_accept(&t, &amal, &contract).await;
    t.on(date(2028, 1, 15), async {
        let response = t
            .post(
                &amal,
                &format!("/manpower/contracts/{}/activate", contract.id),
                &[],
            )
            .await;
        response.assert_status(422);
        assert!(response.text().contains("already ended"));
    })
    .await;
}

async fn sign_and_accept(t: &support::Booted, by: &User, contract: &ClientContract) {
    t.on(date(2027, 1, 10), async {
        sign(t, by, contract).await;
    })
    .await;
    t.cards
        .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
}

/// Base 09: an active contract cannot be activated again, signed again or
/// cancelled: `cancel` is only for a contract never signed into force.
#[renox::test]
async fn only_a_draft_is_activated_signed_or_cancelled() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let active = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let id = active.id;
    t.post(&amal, &format!("/manpower/contracts/{id}/activate"), &[])
        .await
        .assert_status(422);
    t.post(
        &amal,
        &format!("/manpower/contracts/{id}/signed"),
        &[("signed_on", "2027-01-05"), ("document", "x.pdf")],
    )
    .await
    .assert_status(422);
    t.post(
        &amal,
        &format!("/manpower/contracts/{id}/cancel"),
        &[("reason", "changed our mind")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload_contract(id).await.status, ContractStatus::Active);
}

/// Base 09: the signed contract needs a day that has come and a document.
#[renox::test]
async fn a_signature_needs_a_past_day_and_a_document() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let draft = t.contract(TOWER, ContractStatus::Draft, Some(&amal)).await;
    let path = format!("/manpower/contracts/{}/signed", draft.id);
    t.on(date(2027, 1, 10), async {
        t.post(
            &amal,
            &path,
            &[("signed_on", "2027-02-01"), ("document", "x.pdf")],
        )
        .await
        .assert_status(422);
        t.post(
            &amal,
            &path,
            &[("signed_on", "2027-01-05"), ("document", " ")],
        )
        .await
        .assert_status(422);
        t.post(
            &amal,
            &path,
            &[("signed_on", "soon"), ("document", "x.pdf")],
        )
        .await
        .assert_status(422);
    })
    .await;
    assert_eq!(t.reload_contract(draft.id).await.signed_on, None);
}

/// Base 09: `cancel` of a draft keeps the reason and ends it; it needs a
/// reason.
#[renox::test]
async fn a_draft_is_cancelled_with_a_reason() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let draft = t.contract(TOWER, ContractStatus::Draft, Some(&amal)).await;
    let path = format!("/manpower/contracts/{}/cancel", draft.id);
    t.post(&amal, &path, &[("reason", " ")])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload_contract(draft.id).await.status,
        ContractStatus::Draft
    );
    t.post(&amal, &path, &[("reason", "client went elsewhere")])
        .await
        .assert_status(303);
    let cancelled = t.reload_contract(draft.id).await;
    assert_eq!(cancelled.status, ContractStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("client went elsewhere")
    );
    // Nothing leaves a terminal state.
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/activate", draft.id),
        &[],
    )
    .await
    .assert_status(422);
}

/// §3.2: an account manager is assigned per client company, so the contracts
/// of another client company are neither made, signed, activated nor
/// cancelled by them; a role without `contracts.manage` cannot either.
#[renox::test]
async fn an_account_manager_acts_only_in_their_client_companies() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let md = t.md("Mona").await;
    let supervisor = t
        .user_with("Sami", manpower::permissions::SUPERVISOR_ROLE)
        .await;
    let foods = t.contract(FOODS, ContractStatus::Draft, None).await;
    let id = foods.id;
    for user in [&amal, &md, &supervisor] {
        t.post(user, &format!("/manpower/contracts/{id}/activate"), &[])
            .await
            .assert_forbidden();
        t.post(
            user,
            &format!("/manpower/contracts/{id}/signed"),
            &[("signed_on", "2027-01-05"), ("document", "x.pdf")],
        )
        .await
        .assert_forbidden();
        t.post(
            user,
            &format!("/manpower/contracts/{id}/cancel"),
            &[("reason", "no")],
        )
        .await
        .assert_forbidden();
        t.post(
            user,
            "/manpower/contracts",
            &[
                ("reference", "CC-F"),
                ("client_party_id", "32"),
                ("start_date", "2027-01-01"),
                ("end_date", "2027-12-31"),
            ],
        )
        .await
        .assert_forbidden();
    }
    assert_eq!(t.reload_contract(id).await.status, ContractStatus::Draft);
}

/// §3.2: the list shows an account manager only the contracts of their client
/// companies; the MD with `contracts.view` everywhere sees all; a role without
/// the permission is refused. A browser gets the page.
#[renox::test]
async fn the_list_shows_the_contracts_of_the_clients_the_person_works_for() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let md = t.md("Mona").await;
    let nobody = t.user("Nadia").await;
    let tower = t.contract(TOWER, ContractStatus::Active, None).await;
    let foods = t.contract(FOODS, ContractStatus::Draft, None).await;

    let mine: Vec<renox::serde_json::Value> = t.get(&amal, "/manpower/contracts").await.json();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["reference"], tower.reference);
    assert_eq!(mine[0]["client"], "Gulf Tower LLC");
    assert_eq!(mine[0]["status"], "Active");

    let all: Vec<renox::serde_json::Value> = t.get(&md, "/manpower/contracts").await.json();
    let references: Vec<_> = all.iter().map(|row| row["reference"].clone()).collect();
    assert_eq!(
        references,
        [tower.reference.clone(), foods.reference.clone()]
    );

    t.get(&nobody, "/manpower/contracts")
        .await
        .assert_forbidden();

    let page = t.browse(&md, "/manpower/contracts").await;
    page.assert_ok();
    page.assert_see(&tower.reference);
    page.assert_see("Desert Foods LLC");
}

/// A site is added by the account manager of its client only, with a name.
#[renox::test]
async fn a_site_is_added_for_a_client_by_its_account_manager() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let form = |client: &'static str, name: &'static str| {
        vec![
            ("client_party_id", client),
            ("name", name),
            ("address", "Marina"),
        ]
    };
    t.post(&amal, "/manpower/sites", &form("31", "Tower A"))
        .await
        .assert_status(303);
    t.post(&amal, "/manpower/sites", &form("31", " "))
        .await
        .assert_status(422);
    t.post(&amal, "/manpower/sites", &form("32", "Plant 1"))
        .await
        .assert_forbidden();
    let sites = manpower::Site::query().get(t.db()).await.unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].name, "Tower A");
    assert_eq!(sites[0].client_party_id, TOWER.0);
    assert!(sites[0].active);
}
