//! Base 09 (09 Manpower §7.1 *Manpower request*, §3.3 `MPW.ACL.01`): the
//! account manager or the client approver writes a request and `submit`s it;
//! it is `open` once the contract is `active` and the rate card covers the
//! role and the shift; a deployment fills it (`fill_partly`, `fill`); `cancel`
//! needs a reason. The client sees and writes only its own company's
//! requests. The refusal while the contract is on hold is tested with the
//! hold, in BL-MPW-02.

mod support;

use manpower::request::{AUTO_CANCEL, FILL, FILL_PARTLY};
use manpower::{CREDIT_HOLD_TASK, ContractStatus, ManpowerRequest, RequestStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{FOODS, TOWER, boot, date};

/// The form of a request of `contract` for `site`.
fn form(contract: i64, site: i64) -> Vec<(&'static str, String)> {
    vec![
        ("client_contract_id", contract.to_string()),
        ("site_id", site.to_string()),
        ("role", "Guard".into()),
        ("headcount", "2".into()),
        ("shift_type", "Day".into()),
        ("shift_start", "07:00".into()),
        ("shift_end", "19:00".into()),
        ("start_date", "2027-02-01".into()),
        ("end_date", "2027-07-31".into()),
    ]
}

async fn post_form(
    t: &support::Booted,
    by: &User,
    path: &str,
    fields: &[(&'static str, String)],
) -> renox::testing::TestResponse {
    let borrowed: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t.post(by, path, &borrowed).await
}

async fn only_request(t: &support::Booted) -> ManpowerRequest {
    let all = ManpowerRequest::query().get(t.db()).await.unwrap();
    assert_eq!(all.len(), 1);
    all.into_iter().next().unwrap()
}

/// Base 09: the account manager writes a request (a `draft`) and submits it:
/// `open`, with the submitter and the time kept, the rate card checked at the
/// start date, and the account manager of the client told.
#[renox::test]
async fn a_request_written_by_staff_is_submitted_and_opens() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    t.cards
        .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
    post_form(&t, &amal, "/manpower/requests", &form(contract.id, site.id))
        .await
        .assert_status(303);
    let draft = only_request(&t).await;
    assert_eq!(draft.status, RequestStatus::Draft);
    assert_eq!(draft.role, "guard");
    assert_eq!(draft.shift_type, "day");
    assert_eq!(draft.headcount, 2);
    assert_eq!(draft.client_party_id, TOWER.0);
    assert_eq!(draft.filled_count, 0);

    t.on(date(2027, 1, 20), async {
        t.post(
            &amal,
            &format!("/manpower/requests/{}/submit", draft.id),
            &[],
        )
        .await
        .assert_status(303);
    })
    .await;
    let open = t.reload_request(draft.id).await;
    assert_eq!(open.status, RequestStatus::Open);
    assert_eq!(open.submitted_by, Some(amal.id));
    assert!(open.submitted_at.is_some());
    let told = t.notices.of_type("manpower.notice");
    assert_eq!(told.len(), 1);
    assert_eq!(support::users_of(&told[0]), [amal.id]);
    assert!(told[0].data["title"].as_str().unwrap().contains("guard"));
}

/// §3.3 `MPW.ACL.01`: the client approver writes and submits a request for
/// their own company through the portal; a viewer of the same company and an
/// approver of another company cannot.
#[renox::test]
async fn the_client_approver_writes_requests_for_their_own_company_only() {
    let t = boot().await;
    let contract = t.contract(TOWER, ContractStatus::Active, None).await;
    let site = t.site(TOWER, "Tower A").await;
    t.cards
        .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
    let approver = t.client_user("Hana", TOWER, &["approver"]).await;
    let viewer = t.client_user("Vik", TOWER, &["viewer"]).await;
    let other = t.client_user("Omar", FOODS, &["owner"]).await;
    let path = "/manpower/requests";
    let fields = form(contract.id, site.id);
    post_form(&t, &viewer, path, &fields)
        .await
        .assert_forbidden();
    post_form(&t, &other, path, &fields)
        .await
        .assert_forbidden();
    assert!(
        ManpowerRequest::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );

    post_form(&t, &approver, path, &fields)
        .await
        .assert_status(303);
    let draft = only_request(&t).await;
    let submit = format!("/manpower/requests/{}/submit", draft.id);
    t.on(date(2027, 1, 20), async {
        t.post(&viewer, &submit, &[]).await.assert_forbidden();
        t.post(&other, &submit, &[]).await.assert_forbidden();
        t.post(&approver, &submit, &[]).await.assert_status(303);
    })
    .await;
    assert_eq!(t.reload_request(draft.id).await.status, RequestStatus::Open);
}

/// Base 09: a request for a role or a shift the rate card does not price is
/// refused, and every failing check is in the answer.
#[renox::test]
async fn a_request_the_rate_card_does_not_cover_is_not_submitted() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    // The card prices guards by night only.
    t.cards
        .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "night")]);
    let draft = t.request(&contract, &site, 1, RequestStatus::Draft).await;
    t.on(date(2027, 1, 20), async {
        let response = t
            .post(
                &amal,
                &format!("/manpower/requests/{}/submit", draft.id),
                &[],
            )
            .await;
        response.assert_status(422);
        assert!(
            response
                .text()
                .contains("does not price guard on the day shift"),
            "{}",
            response.text()
        );
    })
    .await;
    assert_eq!(
        t.reload_request(draft.id).await.status,
        RequestStatus::Draft
    );
    assert!(t.notices.of_type("manpower.notice").is_empty());
}

/// §7.1 `submit`: a contract that is not `active` takes no request; the
/// refusal to staff names the hold of a contract on hold (BL-MPW-02) and says
/// nothing of a hold otherwise.
#[renox::test]
async fn a_contract_that_is_not_active_takes_no_request() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    // The hold is made by the rule, on 2026-11-05, for INV-1 overdue since 2026-10-01.
    let held = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.invoices
        .issue(&held, "INV-1", 4_200_000, date(2026, 10, 1));
    t.on(date(2026, 11, 5), async {
        t.app.run_scheduled(CREDIT_HOLD_TASK).await.unwrap();
    })
    .await;
    assert_eq!(
        t.reload_contract(held.id).await.status,
        ContractStatus::OnHold
    );
    for status in [
        ContractStatus::Draft,
        ContractStatus::OnHold,
        ContractStatus::Terminating,
    ] {
        let contract = if status == ContractStatus::OnHold {
            t.reload_contract(held.id).await
        } else {
            t.contract(TOWER, status, Some(&amal)).await
        };
        t.cards
            .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
        let draft = t.request(&contract, &site, 1, RequestStatus::Draft).await;
        t.on(date(2027, 1, 20), async {
            let response = t
                .post(
                    &amal,
                    &format!("/manpower/requests/{}/submit", draft.id),
                    &[],
                )
                .await;
            response.assert_status(422);
            let body = response.text().to_lowercase();
            if status == ContractStatus::OnHold {
                // Staff are told of the hold (`MPW.ACL.14`, BL-MPW-02); the
                // client approver is not (see `tests/hold_guards.rs`).
                assert!(
                    body.contains("contract on credit hold since 2026-11-05: overdue (inv-1)"),
                    "{body}"
                );
            } else {
                assert!(body.contains("does not take new requests"), "{body}");
                assert!(!body.contains("hold"), "{body}");
            }
        })
        .await;
        assert_eq!(
            t.reload_request(draft.id).await.status,
            RequestStatus::Draft
        );
    }
}

/// A request that runs past the contract is not submitted.
#[renox::test]
async fn a_request_past_the_end_of_the_contract_is_not_submitted() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    t.cards
        .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
    let mut fields = form(contract.id, site.id);
    fields.retain(|(name, _)| *name != "end_date");
    fields.push(("end_date", "2028-02-28".into()));
    post_form(&t, &amal, "/manpower/requests", &fields)
        .await
        .assert_status(303);
    let draft = only_request(&t).await;
    t.on(date(2027, 1, 20), async {
        let response = t
            .post(
                &amal,
                &format!("/manpower/requests/{}/submit", draft.id),
                &[],
            )
            .await;
        response.assert_status(422);
        assert!(response.text().contains("past the end date"));
    })
    .await;
}

/// A request needs a headcount, a role and a shift, dates in order, and a
/// site of the client that is in use; a contract that is over takes none.
#[renox::test]
async fn a_request_with_missing_or_wrong_details_is_refused() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    let foods_site = t.site(FOODS, "Plant 1").await;
    let path = "/manpower/requests";
    let change = |name: &'static str, value: &str| {
        let mut fields = form(contract.id, site.id);
        fields.retain(|(field, _)| *field != name);
        fields.push((name, value.to_owned()));
        fields
    };
    for fields in [
        change("headcount", "0"),
        change("role", " "),
        change("shift_type", ""),
        change("end_date", "2027-01-01"),
        change("site_id", &foods_site.id.to_string()),
        change("site_id", "999"),
        change("headcount", "many"),
    ] {
        post_form(&t, &amal, path, &fields).await.assert_status(422);
    }
    let over = t.contract(TOWER, ContractStatus::Ended, Some(&amal)).await;
    post_form(&t, &amal, path, &form(over.id, site.id))
        .await
        .assert_status(422);
    post_form(&t, &amal, path, &form(9999, site.id))
        .await
        .assert_status(422);
    assert!(
        ManpowerRequest::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
}

/// Base 09: `cancel` takes a reason, from a draft, an open or a partly filled
/// request, by the account manager or the client approver; a filled or
/// cancelled request cannot be cancelled; a viewer or another company
/// cannot.
#[renox::test]
async fn a_request_is_cancelled_with_a_reason_until_it_is_filled() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    let approver = t.client_user("Hana", TOWER, &["approver"]).await;
    let viewer = t.client_user("Vik", TOWER, &["viewer"]).await;
    let other = t.client_user("Omar", FOODS, &["owner"]).await;
    let cancel = |id: i64| format!("/manpower/requests/{id}/cancel");

    for (status, by) in [
        (RequestStatus::Draft, &amal),
        (RequestStatus::Open, &approver),
        (RequestStatus::PartiallyFilled, &amal),
    ] {
        let request = t.request(&contract, &site, 3, status).await;
        t.post(by, &cancel(request.id), &[("reason", " ")])
            .await
            .assert_status(422);
        t.post(&viewer, &cancel(request.id), &[("reason", "no")])
            .await
            .assert_forbidden();
        t.post(&other, &cancel(request.id), &[("reason", "no")])
            .await
            .assert_forbidden();
        assert_eq!(t.reload_request(request.id).await.status, status);
        t.post(
            by,
            &cancel(request.id),
            &[("reason", "client changed plan")],
        )
        .await
        .assert_status(303);
        let cancelled = t.reload_request(request.id).await;
        assert_eq!(cancelled.status, RequestStatus::Cancelled);
        assert_eq!(
            cancelled.cancel_reason.as_deref(),
            Some("client changed plan")
        );
    }
    for status in [RequestStatus::Filled, RequestStatus::Cancelled] {
        let request = t.request(&contract, &site, 1, status).await;
        t.post(&amal, &cancel(request.id), &[("reason", "late")])
            .await
            .assert_status(422);
        assert_eq!(t.reload_request(request.id).await.status, status);
    }
}

/// Base 09: the system fills a request, one position at a time: `fill_partly`
/// while positions are left, `fill` for the last; a request with no position
/// left or not open takes none.
#[renox::test]
async fn the_system_fills_a_request_one_position_at_a_time() {
    let t = boot().await;
    let contract = t.contract(TOWER, ContractStatus::Active, None).await;
    let site = t.site(TOWER, "Tower A").await;
    let request = t.request(&contract, &site, 2, RequestStatus::Open).await;

    // The last position cannot be taken while two are open, nor the first
    // as a full fill.
    let early = request.transition(FILL).by_system().run(t.state()).await;
    assert!(early.is_err());
    let _first = request
        .transition(FILL_PARTLY)
        .by_system()
        .run(t.state())
        .await
        .unwrap();
    let half = t.reload_request(request.id).await;
    assert_eq!(half.status, RequestStatus::PartiallyFilled);
    assert_eq!(half.filled_count, 1);
    assert_eq!(half.missing(), 1);

    assert!(
        half.transition(FILL_PARTLY)
            .by_system()
            .run(t.state())
            .await
            .is_err()
    );
    let _last = half
        .transition(FILL)
        .by_system()
        .run(t.state())
        .await
        .unwrap();
    let full = t.reload_request(request.id).await;
    assert_eq!(full.status, RequestStatus::Filled);
    assert_eq!(full.filled_count, 2);
    assert!(
        full.transition(FILL)
            .by_system()
            .run(t.state())
            .await
            .is_err()
    );

    let draft = t.request(&contract, &site, 1, RequestStatus::Draft).await;
    assert!(
        draft
            .transition(FILL)
            .by_system()
            .run(t.state())
            .await
            .is_err()
    );
}

/// Base 09: the system cancels a request for the contract's `terminate` or
/// `end` (BL-MPW-04, BL-MPW-06), with a reason; a user's `cancel` and the
/// system's are two transitions.
#[renox::test]
async fn the_system_cancels_a_request_with_a_reason() {
    let t = boot().await;
    let contract = t.contract(TOWER, ContractStatus::Active, None).await;
    let site = t.site(TOWER, "Tower A").await;
    let request = t.request(&contract, &site, 2, RequestStatus::Open).await;
    assert!(
        request
            .transition(AUTO_CANCEL)
            .by_system()
            .run(t.state())
            .await
            .is_err(),
        "a reason is required"
    );
    let _outcome = request
        .transition(AUTO_CANCEL)
        .by_system()
        .reason("contract ended")
        .run(t.state())
        .await
        .unwrap();
    let cancelled = t.reload_request(request.id).await;
    assert_eq!(cancelled.status, RequestStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("contract ended"));
}

/// §3.3 `MPW.ACL.01`: the list shows staff the requests of their client
/// companies and a client user only their own company's, whatever the role in
/// the portal; a user with neither is refused. A browser gets the page.
#[renox::test]
async fn the_list_shows_only_the_requests_of_the_persons_own_company() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let tower = t.contract(TOWER, ContractStatus::Active, None).await;
    let foods = t.contract(FOODS, ContractStatus::Active, None).await;
    let tower_site = t.site(TOWER, "Tower A").await;
    let foods_site = t.site(FOODS, "Plant 1").await;
    let mine = t.request(&tower, &tower_site, 2, RequestStatus::Open).await;
    let _theirs = t.request(&foods, &foods_site, 1, RequestStatus::Open).await;
    let viewer = t.client_user("Vik", TOWER, &["viewer"]).await;
    let nobody = t.user("Nadia").await;

    for user in [&amal, &viewer] {
        let rows: Vec<renox::serde_json::Value> = t.get(user, "/manpower/requests").await.json();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], mine.id);
        assert_eq!(rows[0]["client"], "Gulf Tower LLC");
        assert_eq!(rows[0]["site"], "Tower A");
        assert_eq!(rows[0]["status"], "Open");
    }
    t.get(&nobody, "/manpower/requests")
        .await
        .assert_forbidden();
    let page = t.browse(&viewer, "/manpower/requests").await;
    page.assert_ok();
    page.assert_see("Tower A");
    page.assert_dont_see("Plant 1");
}
