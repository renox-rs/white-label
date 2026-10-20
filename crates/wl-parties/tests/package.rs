//! The party register (wl-parties §3, §4, §6) booted in a `TestApp`: the
//! acceptance checks AC1–AC3 and AC10 of #34.

mod support;

use renox::prelude::*;
use wl_kernel::parties::directory;
use wl_kernel::{PartyKind, PartyStatus};
use wl_parties::{
    BlockReasons, CompanyDetails, ContactPoint, ContactType, NewParty, Party, PersonDetails,
    WlParties, create, permissions as perm,
};

use support::{boot, boot_unseeded, boot_with};

fn email(value: &str) -> ContactPoint {
    ContactPoint {
        contact_type: ContactType::Email,
        value: value.to_owned(),
        is_primary: true,
        ..Default::default()
    }
}

fn person() -> NewParty {
    NewParty {
        kind: PartyKind::Person,
        legal_name_en: "Priya Raghunathan".into(),
        person: Some(PersonDetails::default()),
        contacts: vec![email("priya@mail.example")],
        source: "back_office".into(),
        ..Default::default()
    }
}

fn company() -> NewParty {
    NewParty {
        kind: PartyKind::Company,
        display_name: "Al Noor Trading".into(),
        legal_name_en: "Al Noor Trading LLC".into(),
        company: Some(CompanyDetails {
            vat_registered: true,
            trn: Some("100000000000201".into()),
            ..Default::default()
        }),
        contacts: vec![email("accounts@alnoortrading.example")],
        source: "back_office".into(),
        ..Default::default()
    }
}

/// AC1 (package): `/parties` needs `parties.view`.
#[renox::test]
async fn the_parties_page_needs_the_view_permission() {
    let t = boot().await;
    let nobody = t.user("Nadia", &[]).await;
    t.app.acting_as(&nobody);
    t.app.get("/parties").await.assert_forbidden();

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_view("wl-parties/index.html")
        .assert_see("Al Noor Trading");
}

/// AC2: codes from the series, `active`, one `PartyCreated` each, the TRN
/// and primary e-mail kept, the summary through the kernel.
#[renox::test]
async fn created_parties_get_codes_and_are_found_through_the_kernel() {
    let t = boot_unseeded().await;
    let state = t.app.state();
    let priya = create(state, person()).await.unwrap();
    let al_noor = create(state, company()).await.unwrap();
    assert_eq!(priya.code, "P-00001");
    assert_eq!(al_noor.code, "P-00002");
    assert_eq!(priya.status, PartyStatus::Active);
    assert_eq!(al_noor.status, PartyStatus::Active);

    let created = t.created();
    assert_eq!(created.len(), 2);
    assert_eq!(created[0].party, priya.party_id());
    assert_eq!(created[0].kind, PartyKind::Person);
    assert_eq!(created[1].party, al_noor.party_id());
    assert_eq!(created[1].kind, PartyKind::Company);
    assert_eq!(created[1].source, "back_office");

    let details = CompanyDetails::query()
        .where_(CompanyDetails::PARTY_ID.eq(al_noor.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("company details");
    assert_eq!(details.trn.as_deref(), Some("100000000000201"));
    let contacts = ContactPoint::query()
        .where_(ContactPoint::PARTY_ID.eq(al_noor.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].value, "accounts@alnoortrading.example");
    assert!(contacts[0].is_primary);

    let summary = directory(state)
        .find(t.db(), al_noor.party_id())
        .await
        .unwrap()
        .expect("the kernel finds the party");
    assert_eq!(summary.code, "P-00002");
    assert_eq!(summary.kind, PartyKind::Company);
    assert_eq!(summary.display_name, "Al Noor Trading");
    assert_eq!(summary.status, PartyStatus::Active);
    assert_eq!(summary.display_block_reason, None);
    let by_code = directory(state)
        .find_by_code(t.db(), "P-00001")
        .await
        .unwrap()
        .expect("found by code");
    assert_eq!(by_code.id, priya.party_id());
    // A picker in another module's form searches by name or code, ignoring
    // case, by name, and a blank text lists the first ones.
    let found = directory(state).search(t.db(), "noor", 10).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, al_noor.party_id());
    let by_number = directory(state).search(t.db(), "P-00001", 10).await.unwrap();
    assert_eq!(by_number.len(), 1);
    assert_eq!(by_number[0].id, priya.party_id());
    let all = directory(state).search(t.db(), " ", 10).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(directory(state).search(t.db(), "", 1).await.unwrap().len(), 1);
    assert!(
        directory(state)
            .search(t.db(), "nobody", 10)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC2: a second primary e-mail on one party is refused, by the service and
/// by the table.
#[renox::test]
async fn a_second_primary_email_is_refused() {
    let t = boot_unseeded().await;
    let mut twice = company();
    twice.contacts.push(email("finance@alnoortrading.example"));
    let err = create(t.app.state(), twice).await.unwrap_err();
    assert!(matches!(err, Error::Validation(_)), "{err:?}");
    assert_eq!(Party::query().count(t.db()).await.unwrap(), 0);
    assert!(t.created().is_empty());

    let party = create(t.app.state(), company()).await.unwrap();
    let mut second = email("finance@alnoortrading.example");
    second.party_id = party.id;
    assert!(second.insert(t.db()).await.is_err());
    // Not primary, it is fine.
    let mut other = email("finance@alnoortrading.example");
    other.party_id = party.id;
    other.is_primary = false;
    other.insert(t.db()).await.unwrap();
}

/// AC2: the create form posts a company with its TRN and primary e-mail.
#[renox::test]
async fn the_create_form_makes_a_company() {
    let t = boot_unseeded().await;
    let clerk = t.user("Carla", &[perm::VIEW, perm::CREATE]).await;
    t.app.acting_as(&clerk);
    t.app
        .get("/parties/new")
        .await
        .assert_ok()
        .assert_view("wl-parties/new.html");
    t.app
        .post(
            "/parties",
            &[
                ("kind", "company"),
                ("legal_name_en", "Al Noor Trading LLC"),
                ("display_name", "Al Noor Trading"),
                ("email", "accounts@alnoortrading.example"),
                ("trn", "100000000000201"),
            ],
        )
        .await
        .assert_status(303);
    let party = t.party("Al Noor Trading LLC").await;
    assert_eq!(party.code, "P-00001");
    assert_eq!(party.kind, PartyKind::Company);
    t.app
        .get(&format!("/parties/{}", party.id))
        .await
        .assert_ok()
        .assert_see("100000000000201")
        .assert_see("accounts@alnoortrading.example");
}

/// AC3: a party is inserted only as `active`, and its status changes only
/// through a transition.
#[renox::test]
async fn the_status_moves_only_through_the_machine() {
    let t = boot().await;
    let mut blocked = Party {
        code: "P-09999".into(),
        display_name: "Sneaky".into(),
        legal_name_en: "Sneaky LLC".into(),
        status: PartyStatus::Blocked,
        ..Default::default()
    };
    assert!(blocked.insert(t.db()).await.is_err());
    assert_eq!(
        Party::query()
            .where_(Party::CODE.eq("P-09999"))
            .count(t.db())
            .await
            .unwrap(),
        0
    );

    let mut al_noor = t.party("Al Noor Trading LLC").await;
    al_noor.status = PartyStatus::Blocked;
    assert!(al_noor.save(t.db()).await.is_err());
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Active);
}

/// AC10: the seven defaults, two of them restricted.
#[renox::test]
async fn the_app_has_the_seven_default_reasons() {
    let t = boot().await;
    let reasons = BlockReasons::of(t.app.state());
    let codes: Vec<_> = reasons.all().iter().map(|r| r.code.as_str()).collect();
    assert_eq!(
        codes,
        [
            "on_hold",
            "non_payment",
            "legal_dispute",
            "fraud_suspected",
            "other",
            "sanctions",
            "aml_concern"
        ]
    );
    let restricted: Vec<_> = reasons
        .all()
        .iter()
        .filter(|r| r.restricted)
        .map(|r| r.code.as_str())
        .collect();
    assert_eq!(restricted, ["sanctions", "aml_concern"]);
}

/// AC10: a vertical's restricted code is hidden like the defaults.
#[renox::test]
async fn a_verticals_restricted_code_is_hidden_like_the_defaults() {
    let t = boot_with(
        WlParties::new()
            .block_reason("tax_evasion", true)
            .block_reason("unpaid_deposit", false),
        true,
    )
    .await;
    let reasons = BlockReasons::of(t.app.state());
    assert!(reasons.is_restricted("tax_evasion"));
    assert!(!reasons.is_restricted("unpaid_deposit"));

    let al_noor = t.party("Al Noor Trading LLC").await;
    let b = t.user_b().await;
    t.app.acting_as(&b);
    let page = format!("/parties/{}", al_noor.id);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see(r#"value="unpaid_deposit""#)
        .assert_dont_see("tax_evasion");
    // A forged post with it is refused.
    t.app
        .htmx()
        .post(
            &format!("{page}/block"),
            &[("reason_code", "tax_evasion"), ("note", "suspected")],
        )
        .await
        .assert_invalid("reason_code");
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Active);

    let mlro = t.mlro().await;
    t.app.acting_as(&mlro);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see(r#"value="tax_evasion""#);
    t.app
        .htmx()
        .post(
            &format!("{page}/block"),
            &[("reason_code", "tax_evasion"), ("note", "suspected")],
        )
        .await
        .assert_ok();
    let blocked = t.reload(&al_noor).await;
    assert_eq!(blocked.status, PartyStatus::Blocked);
    assert_eq!(blocked.hold_reason.as_deref(), Some("on_hold"));
    assert_eq!(t.blocked()[0].display_reason, "on_hold");
}

/// AC10: `on_hold` cannot be made restricted; the app is not built.
#[test]
#[should_panic(expected = "on_hold")]
fn restricting_on_hold_stops_the_boot() {
    let _ = App::new().module(WlParties::new().block_reason("on_hold", true));
}

/// AC10: `on_hold` cannot be redefined (and so not removed) either.
#[test]
#[should_panic(expected = "on_hold")]
fn redefining_on_hold_stops_the_boot() {
    let _ = App::new().module(WlParties::new().block_reason("on_hold", false));
}

#[renox::test]
async fn the_models_match_the_schema() {
    boot().await.app.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    boot().await.app.assert_views_compile();
}
