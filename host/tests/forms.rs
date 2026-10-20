//! Forms sent with htmx show what went wrong: field errors next to the field
//! (a malformed value, a tenant that does not exist), and a business-rule
//! refusal as a message the page script puts in the modal (the compliance
//! block of a lease). The tenant field takes the party number shown in the
//! UI, and the picker behind it searches the parties.

mod support;

use host::roles::LEASING_AGENT;
use property::UnitUse;
use support::*;

const LEASE: &[(&str, &str)] = &[
    ("start_date", "2026-12-01"),
    ("end_date", "2027-11-30"),
    ("annual_rent", "90000"),
    ("instalments", "4"),
];

fn with<'a>(tenant: &'a str, rest: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut form = vec![("tenant", tenant)];
    form.extend_from_slice(rest);
    form
}

#[renox::test]
async fn the_lease_form_takes_the_party_number_and_the_id_and_refuses_a_wrong_one_at_the_field() {
    let app = booted().await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let tara = tenant(&app, "Tara Nasser").await;
    let scope = property::Building::scope_of(unit.building_id);
    let agent = scoped_person(&app, "Ali", LEASING_AGENT, &scope).await;
    app.acting_as(&agent);
    let url = format!("/property/units/{}/leases", unit.id);

    // The number the UI shows (any case, with or without the zeros), the id.
    for (n, typed) in [
        format!("P-{:05}", tara.0),
        format!("p-{}", tara.0),
        tara.0.to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        // A unit has one open lease: each way of typing gets its own unit.
        let other = unit_of(&app, &format!("TWR-{n}"), "1", UnitUse::Residential).await;
        let scope = property::Building::scope_of(other.building_id);
        let agent = scoped_person(&app, &format!("Agent{n}"), LEASING_AGENT, &scope).await;
        app.acting_as(&agent);
        let form = with(&typed, LEASE);
        let done = app
            .htmx()
            .post(&format!("/property/units/{}/leases", other.id), &form)
            .await;
        assert_eq!(done.status.as_u16(), 200, "tenant `{typed}`");
        assert!(
            done.header("HX-Redirect")
                .is_some_and(|to| to.starts_with("/property/leases/"))
        );
    }
    app.acting_as(&agent);

    // A tenant that does not exist is an error on the tenant field.
    for wrong in ["P-99999", "12345678", "", "tara"] {
        let form = with(wrong, LEASE);
        app.htmx()
            .post(&url, &form)
            .await
            .assert_status(422)
            .assert_invalid("tenant");
    }
    // A malformed value is an error on its own field.
    let form = with(
        "1",
        &[
            ("start_date", "soon"),
            ("end_date", "2027-11-30"),
            ("annual_rent", "90000"),
            ("instalments", "4"),
        ],
    );
    app.htmx()
        .post(&url, &form)
        .await
        .assert_invalid("start_date");
    let form = with(
        "1",
        &[
            ("start_date", "2026-12-01"),
            ("end_date", "2027-11-30"),
            ("annual_rent", "lots"),
            ("instalments", "4"),
        ],
    );
    app.htmx()
        .post(&url, &form)
        .await
        .assert_invalid("annual_rent");
    let form = with(
        "1",
        &[
            ("start_date", "2026-12-01"),
            ("end_date", "2027-11-30"),
            ("annual_rent", "90000"),
            ("instalments", "four"),
        ],
    );
    app.htmx()
        .post(&url, &form)
        .await
        .assert_invalid("instalments");
}

#[renox::test]
async fn the_tenant_picker_searches_the_parties_by_name_or_number() {
    let app = booted().await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    let tara = tenant(&app, "Tara Nasser").await;
    let _omar = tenant(&app, "Omar Haddad").await;
    let scope = property::Building::scope_of(unit.building_id);
    let agent = scoped_person(&app, "Ali", LEASING_AGENT, &scope).await;
    app.acting_as(&agent);

    let found = app.get("/property/parties/options?q=nasser").await;
    found.assert_ok();
    let options: Vec<renox::serde_json::Value> = found.json();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0]["value"], tara.0.to_string());
    assert_eq!(
        options[0]["label"],
        format!("P-{:05} · Tara Nasser", tara.0)
    );
    // By number, and the label of a value the form already holds.
    let by_number: Vec<renox::serde_json::Value> = app
        .get(&format!("/property/parties/options?q=P-{:05}", tara.0))
        .await
        .json();
    assert_eq!(by_number.len(), 1);
    let held: Vec<renox::serde_json::Value> = app
        .get(&format!("/property/parties/options?values={}", tara.0))
        .await
        .json();
    assert_eq!(held[0]["label"], format!("P-{:05} · Tara Nasser", tara.0));

    // Someone who may not draft a lease has no list of the parties.
    let member = person(&app, "Mina", host::roles::MEMBER).await;
    app.acting_as(&member);
    app.get("/property/parties/options?q=a")
        .await
        .assert_forbidden();
}

#[renox::test]
async fn a_refused_submit_carries_its_message_for_the_modal() {
    let app = booted().await;
    let unit = unit_of(&app, "TWR-A", "1204", UnitUse::Residential).await;
    // A tenant with no identity documents: the compliance block refuses.
    let tara = tenant(&app, "Tara Nasser").await;
    let lease = draft_lease(&app, &unit, tara, 10).await;
    let scope = property::Building::scope_of(unit.building_id);
    let agent = scoped_person(&app, "Ali", LEASING_AGENT, &scope).await;
    app.acting_as(&agent);
    let refused = app
        .htmx()
        .post(&format!("/property/leases/{}/submit", lease.id), &[])
        .await;
    refused.assert_status(422);
    let html = refused.text();
    // The error page names its message in `data-error-detail`, which
    // public/forms.js shows as an alert in the modal that was submitted.
    assert!(
        html.contains("data-error-detail=\"The tenant needs a valid Emirates ID, passport and residence visa on file."),
        "the page carries the refusal"
    );
    // The script that does it is loaded by the layout.
    assert!(html.contains("forms.js"));
}
