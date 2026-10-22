//! BL-MPW-05 (#419), `renew` and `MPW.ACL.16`, with the real wl-invoicing in
//! the host of a manpower agency: a renewal that changes the rates needs a new
//! rate card version of the contract, accepted by the client and effective from
//! the date given. A pending version does not count; the version in force is
//! never edited.

mod support;

use host::roles::{ACCOUNT_MANAGER, ADMIN};
use manpower::ClientContract;
use manpower::ContractStatus;
use manpower::permissions::client_scope;
use renox::chrono::Days;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, noon_clock, person, scoped_person};
use wl_invoicing::rate_card::{RateCardVersion, Unit, VersionStatus};
use wl_invoicing::rate_cards::{self, LineInput};
use wl_kernel::ClientConfig;
use wl_kernel::documents::today;

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

/// AC4: the contract (60 days old, ending in 30) is renewed by Amal. The renewal
/// changes the rates from today + 100 days: refused while there is no version
/// for that date, refused while the version is pending, allowed once the client
/// accepted it (staff record the acceptance). The first version keeps its rate.
#[renox::test]
async fn a_renewal_with_changed_rates_needs_an_accepted_rate_card_version_of_the_date() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    let rana = person(&app, "Rana", ADMIN).await;
    let client = company(&app, "Gulf Tower LLC").await;
    let amal = scoped_person(&app, "Amal", ACCOUNT_MANAGER, &client_scope(client.0)).await;
    let day = |days: i64| {
        today()
            .checked_add_days(Days::new(days.unsigned_abs()))
            .unwrap()
    };
    let start = today().checked_sub_days(Days::new(60)).unwrap();
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: client.0,
        start_date: start,
        end_date: day(30),
        payment_terms_days: 30,
        notice_period_days: 30,
        account_manager_user_id: Some(amal.id),
        status: ContractStatus::Active,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let card = rate_cards::create_card(app.db(), None, client, &contract.subject())
        .await
        .unwrap();
    let guard = |rate: i64| LineInput::new("guard", "day", Unit::Hour, rate * 100);
    let accept = |version: &RateCardVersion| {
        let (app, rana) = (&app, &rana);
        let path = format!("/rate-card-versions/{}/accept", version.id);
        async move {
            app.acting_as(rana);
            app.htmx()
                .post(&path, &[("document", "signed-acceptance.pdf")])
                .await
        }
    };
    let v1 = rate_cards::new_version(app.db(), None, card.id, start, false)
        .await
        .unwrap();
    rate_cards::add_line(app.db(), v1.id, guard(25))
        .await
        .unwrap();
    accept(&v1).await.assert_status(303);

    let new_end = day(395).to_string();
    let rates_from = day(100).to_string();
    let signed_on = start.to_string();
    let form = [
        ("end_date", new_end.as_str()),
        ("signed_on", signed_on.as_str()),
        ("document", "RENEWAL.pdf"),
        ("rates_from", rates_from.as_str()),
    ];
    let path = format!("/manpower/contracts/{}/renew", contract.id);
    let renew = || {
        let (app, amal, path, form) = (&app, &amal, &path, &form);
        async move {
            app.acting_as(amal);
            app.htmx().post(path, form).await
        }
    };

    // No version for the date: refused, and the contract is unchanged.
    let refused = renew().await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("rate card version"),
        "{}",
        refused.text()
    );
    let kept = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(kept.end_date, day(30));

    // A version made but still pending prices nothing: refused.
    let v2 = rate_cards::new_version(app.db(), None, card.id, day(100), false)
        .await
        .unwrap();
    rate_cards::add_line(app.db(), v2.id, guard(27))
        .await
        .unwrap();
    renew().await.assert_status(422);

    // The client accepts V2 (dated in the future: no approval is needed).
    accept(&v2).await.assert_status(303);
    let accepted = RateCardVersion::find_or_404(app.db(), v2.id).await.unwrap();
    assert_eq!(accepted.status, VersionStatus::Accepted);
    renew().await.assert_status(303);

    let renewed = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(renewed.status, ContractStatus::Active);
    assert_eq!(renewed.end_date, day(395));
    assert_eq!(renewed.renewal_rates_from, Some(day(100)));
    // The first version was not edited: its guard rate is still AED 25 an hour.
    let v1 = RateCardVersion::find_or_404(app.db(), v1.id).await.unwrap();
    let lines = v1.lines(app.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].rate, 2_500);
}
