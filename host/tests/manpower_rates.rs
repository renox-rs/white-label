//! BL-MPW-07 (#421), `MPW.ACL.16`, through the real host of a manpower
//! agency: the rate cards are those of wl-invoicing, so a contract is
//! activated and a request submitted only once a rate card version the client
//! accepted is in force and prices the role and the shift.

use host::roles::ADMIN;
use manpower::{ClientContract, ContractStatus, ManpowerRequest, RequestStatus, Site};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::rate_card::{RateCardVersion, Unit, VersionStatus};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_kernel::{ClientConfig, PartyId};

/// A manpower agency: the vertical on, with the shared modules it reaches.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// The MD (the company admin), signed in.
async fn md(app: &TestApp) -> User {
    app.kernel().seed().await.unwrap();
    let mut admin = User::register(app.db(), "Rana", "rana@example.com", "secret123")
        .await
        .unwrap();
    admin.email_verified_at = Some(renox::db::now());
    admin.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    admin.assign_role(app.db(), ADMIN).await.unwrap();
    admin
}

/// `POST path` as `by`.
async fn post(
    app: &TestApp,
    by: &User,
    path: &str,
    form: &[(&str, &str)],
) -> renox::testing::TestResponse {
    app.acting_as(by);
    app.htmx().post(path, form).await
}

/// Done when 1 of the base: a signed contract is refused while no rate card
/// version is accepted and activates once the client accepted one that is in
/// force on the start date; a request opens only for a role and shift the
/// card prices.
#[renox::test]
async fn the_wl_invoicing_rate_card_lets_the_contract_activate_and_the_request_submit() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    let rana = md(&app).await;
    let contract = ClientContract::create(
        app.db(),
        ClientContract {
            reference: "CC-0001".into(),
            client_party_id: 3,
            start_date: date(2027, 1, 1),
            end_date: date(2027, 12, 31),
            payment_terms_days: 30,
            notice_period_days: 30,
            ..ClientContract::default()
        },
    )
    .await
    .unwrap();
    let id = contract.id;
    post(
        &app,
        &rana,
        &format!("/manpower/contracts/{id}/signed"),
        &[("signed_on", "2026-10-01"), ("document", "contract.pdf")],
    )
    .await
    .assert_status(303);

    // No rate card yet: refused.
    let refused = post(&app, &rana, &format!("/manpower/contracts/{id}/activate"), &[]).await;
    refused.assert_status(422);

    // A pending version is not enough.
    let card = rate_cards::create_card(app.db(), None, PartyId(3), &contract.subject())
        .await
        .unwrap();
    let version = rate_cards::new_version(app.db(), None, card.id, date(2027, 1, 1), false)
        .await
        .unwrap();
    rate_cards::add_line(
        app.db(),
        version.id,
        LineInput::new("guard", "day", Unit::Hour, 2_500),
    )
    .await
    .unwrap();
    post(&app, &rana, &format!("/manpower/contracts/{id}/activate"), &[])
        .await
        .assert_status(422);

    // The client's acceptance, recorded by the MD with the signed document.
    post(
        &app,
        &rana,
        &format!("/rate-card-versions/{}/accept", version.id),
        &[("document", "rate-card-v1.pdf")],
    )
    .await
    .assert_status(303);
    let accepted = RateCardVersion::find_or_404(app.db(), version.id).await.unwrap();
    assert_eq!(accepted.status, VersionStatus::Accepted);
    post(&app, &rana, &format!("/manpower/contracts/{id}/activate"), &[])
        .await
        .assert_status(303);
    let active = ClientContract::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(active.status, ContractStatus::Active);

    // A request for a role the card prices opens; one it does not stays a draft.
    let site = Site::create(
        app.db(),
        Site {
            client_party_id: 3,
            name: "Tower A".into(),
            active: true,
            ..Site::default()
        },
    )
    .await
    .unwrap();
    let mut requests = Vec::new();
    for role in ["guard", "cleaner"] {
        requests.push(
            ManpowerRequest::create(
                app.db(),
                ManpowerRequest {
                    client_contract_id: id,
                    client_party_id: 3,
                    site_id: site.id,
                    role: role.into(),
                    headcount: 1,
                    shift_type: "day".into(),
                    start_date: date(2027, 2, 1),
                    end_date: Some(date(2027, 7, 31)),
                    ..ManpowerRequest::default()
                },
            )
            .await
            .unwrap(),
        );
    }
    post(
        &app,
        &rana,
        &format!("/manpower/requests/{}/submit", requests[0].id),
        &[],
    )
    .await
    .assert_status(303);
    post(
        &app,
        &rana,
        &format!("/manpower/requests/{}/submit", requests[1].id),
        &[],
    )
    .await
    .assert_status(422);
    let status = |n: usize| {
        let id = requests[n].id;
        let db = app.db().clone();
        async move { ManpowerRequest::find_or_404(&db, id).await.unwrap().status }
    };
    assert_eq!(status(0).await, RequestStatus::Open);
    assert_eq!(status(1).await, RequestStatus::Draft);
}
