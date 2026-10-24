//! The lists of the Clinics vertical on the demo clinic (#535): each is a
//! `renox::grid` that is a direct child of the page (it fills the screen),
//! searches (also the patient's and the payer's names, which live in the
//! parties module), filters by its columns (the payer, the open flag, the
//! branch, the policy), sorts and pages through the query string, and
//! exports for the roles that may.

mod grid_support;

use grid_support::{encode, grid_is_a_child_of_the_page, load, range, total};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Clinics vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-clinics/client.toml"
);

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn text_of(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap()
}

/// The demo clinic, seeded, with the administrator signed in.
async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Clinics vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@palmgrove.example")
        .await
        .unwrap()
        .expect("the demo administrator");
    app.acting_as(&admin);
    app
}

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_clinics_list_is_a_grid_that_fills_the_page() {
    for template in [
        "clinics/views/clinics/appointments.html",
        "clinics/views/clinics/claims.html",
        "clinics/views/clinics/policies.html",
        "clinics/views/clinics/authorisations.html",
        "clinics/views/clinics/contracts.html",
        "clinics/views/clinics/call_list.html",
        "clinics/views/clinics/recall_calls.html",
        "clinics/views/clinics/permits.html",
    ] {
        grid_is_a_child_of_the_page(template);
    }
}

/// The claims: page size, sort, pages, a filter by payer, the search on the
/// patient's name and the export.
#[renox::test]
async fn the_claims_page_sort_filter_search_and_export() {
    let app = demo().await;
    let claims = count(&app, "select count(*) from clinics_claims").await;
    assert!(claims >= 2, "the demo has {claims} claims");
    let first = count(&app, "select min(id) from clinics_claims").await;
    let last = count(&app, "select max(id) from clinics_claims").await;
    let number = |id: i64| format!(">CLM-{id:04}<");

    let all = load(&app, "/clinics/claims?per_page=10&sort=id").await;
    assert_eq!(range(&all), (1, claims.min(10), claims));
    assert!(all.find(&number(first)).unwrap() < all.find(&number(last)).unwrap());
    let newest = load(&app, "/clinics/claims?per_page=10&sort=-id").await;
    assert!(newest.find(&number(last)).unwrap() < newest.find(&number(first)).unwrap());

    // A filter by payer counts the payer's claims.
    let payer = count(&app, "select min(payer_party_id) from clinics_claims").await;
    let of_payer = count(
        &app,
        &format!("select count(*) from clinics_claims where payer_party_id = {payer}"),
    )
    .await;
    assert_eq!(
        total(&app, &format!("/clinics/claims?in.payer_party_id={payer}")).await,
        of_payer
    );

    // The patient is found by name, though the name is in the parties module.
    let patient = text_of(
        &app,
        &format!(
            "select display_name from wl_parties_parties where id = \
             (select patient_party_id from clinics_claims where id = {first})"
        ),
    )
    .await;
    let found = load(
        &app,
        &format!("/clinics/claims?search={}", encode(&patient)),
    )
    .await;
    let (_, _, hits) = range(&found);
    assert!(hits >= 1 && hits <= claims, "{patient}: {hits} claims");
    assert!(found.contains(&number(first)), "{patient}'s claim is found");
    let none = load(&app, "/clinics/claims?search=zzqqxx-nobody").await;
    assert_eq!(range(&none).2, 0);

    // The billing staff may export.
    let file = app
        .request()
        .header("accept", "text/html")
        .get("/clinics/claims?export=csv")
        .await;
    assert_eq!(file.status.as_u16(), 200);
}

/// The policies and the authorisations filter by payer and policy and find
/// the patient by name; the contracts filter by payer.
#[renox::test]
async fn the_policies_authorisations_and_contracts_filter_by_payer() {
    let app = demo().await;
    let policies = count(&app, "select count(*) from clinics_insurance_policies").await;
    assert_eq!(total(&app, "/clinics/policies?per_page=10").await, policies);
    let payer = count(
        &app,
        "select min(payer_party_id) from clinics_insurance_policies",
    )
    .await;
    let of_payer = count(
        &app,
        &format!("select count(*) from clinics_insurance_policies where payer_party_id = {payer}"),
    )
    .await;
    assert_eq!(
        total(
            &app,
            &format!("/clinics/policies?in.payer_party_id={payer}")
        )
        .await,
        of_payer
    );
    let member = text_of(
        &app,
        "select member_id from clinics_insurance_policies order by id limit 1",
    )
    .await;
    let found = total(
        &app,
        &format!("/clinics/policies?search={}", encode(&member)),
    )
    .await;
    assert!(found >= 1, "the member id {member} is found");
    let patient = text_of(
        &app,
        "select display_name from wl_parties_parties where id = \
         (select patient_party_id from clinics_insurance_policies order by id limit 1)",
    )
    .await;
    let found = total(
        &app,
        &format!("/clinics/policies?search={}", encode(&patient)),
    )
    .await;
    assert!(found >= 1, "the patient {patient} is found");

    let authorisations = count(&app, "select count(*) from clinics_prior_authorisations").await;
    assert_eq!(
        total(&app, "/clinics/authorisations?per_page=10").await,
        authorisations
    );
    if authorisations > 0 {
        let policy = count(
            &app,
            "select min(policy_id) from clinics_prior_authorisations",
        )
        .await;
        let of_policy = count(
            &app,
            &format!(
                "select count(*) from clinics_prior_authorisations where policy_id = {policy}"
            ),
        )
        .await;
        assert_eq!(
            total(
                &app,
                &format!("/clinics/authorisations?in.policy_id={policy}")
            )
            .await,
            of_policy
        );
        let first = count(&app, "select min(id) from clinics_prior_authorisations").await;
        let found = load(
            &app,
            &format!("/clinics/authorisations?search=PA-{first:04}"),
        )
        .await;
        assert!(found.contains(&format!(">PA-{first:04}<")));
    }

    let contracts = count(&app, "select count(*) from clinics_payer_contracts").await;
    assert_eq!(
        total(&app, "/clinics/contracts?per_page=10").await,
        contracts
    );
    let payer = count(
        &app,
        "select min(payer_party_id) from clinics_payer_contracts",
    )
    .await;
    let of_payer = count(
        &app,
        &format!("select count(*) from clinics_payer_contracts where payer_party_id = {payer}"),
    )
    .await;
    assert_eq!(
        total(
            &app,
            &format!("/clinics/contracts?in.payer_party_id={payer}")
        )
        .await,
        of_payer
    );
}

/// The appointments: the flag filter splits the list, pages hold ten rows,
/// and the patient is found by name.
#[renox::test]
async fn the_appointments_filter_by_flag_page_and_find_the_patient() {
    let app = demo().await;
    let all = load(&app, "/clinics/appointments?per_page=10").await;
    let (from, to, appointments) = range(&all);
    if appointments == 0 {
        return;
    }
    assert_eq!((from, to), (1, appointments.min(10)));
    if appointments > 10 {
        let second = load(&app, "/clinics/appointments?per_page=10&page=2").await;
        assert_eq!(range(&second), (11, appointments.min(20), appointments));
    }
    let mut split = 0;
    for flag in ["none", "needs_authorisation", "insurance_card"] {
        split += total(&app, &format!("/clinics/appointments?in.flags={flag}")).await;
    }
    assert_eq!(split, appointments, "every appointment has one flag value");
    let patient = text_of(
        &app,
        "select p.display_name from wl_parties_parties p \
         join clinics_appointments a on a.patient_party_id = p.id \
         where a.cancelled_on is null order by a.starts_at desc limit 1",
    )
    .await;
    let found = total(
        &app,
        &format!("/clinics/appointments?search={}", encode(&patient)),
    )
    .await;
    assert!(found >= 1, "the patient {patient} is found");
    assert_eq!(
        total(&app, "/clinics/appointments?search=zzqqxx-nobody").await,
        0
    );
}

/// The recall calls filter by branch; the permits export for who records
/// them.
#[renox::test]
async fn the_recall_calls_filter_by_branch_and_the_permits_export() {
    let app = demo().await;
    let calls = total(&app, "/clinics/recall-calls").await;
    if calls > 0 {
        let first = total(&app, "/clinics/recall-calls?in.branch=k1").await;
        assert!(first >= 1 && first <= calls);
    }
    let permits = count(&app, "select count(*) from clinics_advertising_permits").await;
    assert_eq!(total(&app, "/clinics/campaign-permits").await, permits);
    let file = app
        .request()
        .header("accept", "text/html")
        .get("/clinics/campaign-permits?export=csv")
        .await;
    assert_eq!(file.status.as_u16(), 200);
}
