//! BL-PTY-10 (#43): a company's e-invoicing recipient identifier. It is
//! exposed through the `PartyDirectory`, a change emits `PartyUpdated` with
//! the field name only, and *Look up* by TRN asks the ASP adapter, fills the
//! field for the user to confirm and saves nothing. The lookup is off while
//! the ASP connection is not `up`.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::einvoice::{Answer, LookupFuture, PARTY_SCREEN, RecipientLookup};
use wl_kernel::integrations::{self, Delivery, IntegrationDef};
use wl_kernel::parties::PartyUpdated;
use wl_kernel::{LegalEntity, PartyId, PartyKind};
use wl_parties::create::{NewParty, create};
use wl_parties::{CompanyDetails, Party, WlParties, permissions as perm};

use support::{Booted, boot_custom};

const KEY: &str = "asp.pgp";
const DELTA: &str = "Delta Trading LLC";
const DELTA_TRN: &str = "100123456700003";

/// What the fake ASP answers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reply {
    Found,
    NotFound,
    Error,
}

/// A fake ASP adapter for the default legal entity: answers `0235:<trn>`.
#[derive(Clone)]
struct Asp {
    reply: Arc<Mutex<Reply>>,
    calls: Arc<AtomicUsize>,
}

impl Asp {
    fn say(&self, reply: Reply) {
        *self.reply.lock().unwrap() = reply;
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RecipientLookup for Asp {
    fn connection(&self, _entity: &LegalEntity) -> Option<String> {
        Some(KEY.to_owned())
    }

    fn lookup<'a>(
        &'a self,
        _state: &'a AppState,
        _entity: &'a LegalEntity,
        trn: &'a str,
    ) -> LookupFuture<'a, Answer> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let reply = *self.reply.lock().unwrap();
        Box::pin(async move {
            match reply {
                Reply::Found => Ok(Answer::Found(format!("0235:{trn}"))),
                Reply::NotFound => Ok(Answer::NotFound),
                Reply::Error => Err(integrations::call_failed("the ASP does not answer")),
            }
        })
    }
}

/// The app with the fake ASP, its connection `up`, and events recorded.
async fn setup() -> (Booted, Asp) {
    IntegrationDef::of(KEY, "invoicing", |_state, _call| {
        Box::pin(async { Ok(Delivery::Sent) })
    })
    .screens([PARTY_SCREEN])
    .fallback("Enter the e-invoicing ID by hand.", None)
    .register();
    let asp = Asp {
        reply: Arc::new(Mutex::new(Reply::Found)),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let provided = asp.clone();
    let t = boot_custom(WlParties::new(), false, move |app| {
        let lookup: Arc<dyn RecipientLookup> = Arc::new(provided);
        app.provide(lookup)
    })
    .await;
    t.app.fake_events();
    (t, asp)
}

async fn company(t: &Booted, name: &str, trn: Option<&str>, recipient: Option<&str>) -> Party {
    create(
        t.app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: name.into(),
            company: Some(CompanyDetails {
                vat_registered: trn.is_some(),
                trn: trn.map(str::to_owned),
                einvoice_recipient_id: recipient.map(str::to_owned),
                ..Default::default()
            }),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

async fn stored(t: &Booted, party: &Party) -> Option<String> {
    CompanyDetails::query()
        .where_(CompanyDetails::PARTY_ID.eq(party.id))
        .first(t.db())
        .await
        .unwrap()
        .unwrap()
        .einvoice_recipient_id
}

async fn identity_id(t: &Booted, party: &Party) -> Option<String> {
    wl_kernel::parties::directory(t.app.state())
        .billing_identity(t.db(), PartyId(party.id))
        .await
        .unwrap()
        .unwrap()
        .einvoice_recipient_id
}

/// AC1: a company saved with `0235:1234567890` exposes it through the
/// `PartyDirectory`, trimmed.
#[renox::test]
async fn ptyeinv01_the_directory_exposes_the_recipient_identifier() {
    let (t, _asp) = setup().await;
    let party = company(&t, DELTA, Some(DELTA_TRN), Some("  0235:1234567890 ")).await;
    assert_eq!(
        identity_id(&t, &party).await.as_deref(),
        Some("0235:1234567890")
    );

    let other = company(&t, "Sigma Trading LLC", None, None).await;
    assert_eq!(identity_id(&t, &other).await, None);
}

/// Saves `value` on the e-invoicing page of `party` as the acting user.
async fn save(t: &Booted, party: &Party, value: &str) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(
            &format!("/parties/{}/einvoice", party.id),
            &[("einvoice_recipient_id", value)],
        )
        .await
}

/// AC2: changing it emits `PartyUpdated` with the field name only, once per
/// save, and the value is not in the payload; saving the same value emits
/// nothing.
#[renox::test]
async fn ptyeinv02_a_change_emits_party_updated_with_the_field_name_only() {
    let (t, _asp) = setup().await;
    let party = company(&t, DELTA, Some(DELTA_TRN), None).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);

    save(&t, &party, " 0235:1234567890 ")
        .await
        .assert_redirect(&format!("/parties/{}", party.id));
    assert_eq!(stored(&t, &party).await.as_deref(), Some("0235:1234567890"));
    let events = t.app.emitted::<PartyUpdated>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].party, PartyId(party.id));
    assert_eq!(events[0].changed_fields, ["einvoice_recipient_id"]);
    let json = renox::serde_json::to_string(&events[0]).unwrap();
    assert!(!json.contains("1234567890"), "{json}");

    save(&t, &party, "0235:1234567890").await;
    assert_eq!(t.app.emitted::<PartyUpdated>().len(), 1);

    save(&t, &party, "0235:999").await;
    assert_eq!(t.app.emitted::<PartyUpdated>().len(), 2);
    assert_eq!(identity_id(&t, &party).await.as_deref(), Some("0235:999"));
}

/// A person has no such field, and it needs `parties.update`.
#[renox::test]
async fn ptyeinv03_a_person_has_no_field_and_the_change_needs_update() {
    let (t, _asp) = setup().await;
    let person = create(
        t.app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: "Khalid Saeed".into(),
            person: Some(Default::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let party = company(&t, DELTA, Some(DELTA_TRN), None).await;

    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    save(&t, &party, "0235:1").await.assert_status(403);
    assert_eq!(stored(&t, &party).await, None);

    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    save(&t, &person, "0235:1").await.assert_status(422);
    t.app
        .get(&format!("/parties/{}/einvoice", person.id))
        .await
        .assert_not_found();
    assert!(t.app.emitted::<PartyUpdated>().is_empty());
}

/// AC3: with the ASP returning `0235:100123456700003`, *Look up* fills the
/// field, and nothing is saved until the user confirms (saves the form).
#[renox::test]
async fn ptyeinv04_look_up_fills_the_field_and_saves_nothing_until_confirmed() {
    let (t, asp) = setup().await;
    let party = company(&t, DELTA, Some(DELTA_TRN), None).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let page = format!("/parties/{}/einvoice", party.id);

    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Look up")
        .assert_dont_see("The lookup is not available now");

    let found = "0235:100123456700003";
    t.app
        .htmx()
        .post(&format!("{page}/lookup"), &[])
        .await
        .assert_redirect(&format!("{page}?result=found&found=0235%3A100123456700003"));
    assert_eq!(asp.calls(), 1);
    assert_eq!(stored(&t, &party).await, None);
    assert!(t.app.emitted::<PartyUpdated>().is_empty());

    t.app
        .get(&format!("{page}?result=found&found=0235%3A100123456700003"))
        .await
        .assert_ok()
        .assert_see(&format!(r#"value="{found}""#));
    assert_eq!(stored(&t, &party).await, None);

    t.app
        .htmx()
        .post(&page, &[("einvoice_recipient_id", found)])
        .await
        .assert_redirect(&format!("/parties/{}", party.id));
    assert_eq!(stored(&t, &party).await.as_deref(), Some(found));
    assert_eq!(t.app.emitted::<PartyUpdated>().len(), 1);
}

/// AC4: when the ASP answers "not found", the field stays empty and the
/// message asks for entry by hand; an error does the same and is reported to
/// the connection.
#[renox::test]
async fn ptyeinv05_not_found_or_an_error_leaves_the_field_empty() {
    let (t, asp) = setup().await;
    let party = company(&t, DELTA, Some(DELTA_TRN), None).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let page = format!("/parties/{}/einvoice", party.id);

    asp.say(Reply::NotFound);
    t.app
        .htmx()
        .post(&format!("{page}/lookup"), &[])
        .await
        .assert_redirect(&format!("{page}?result=not_found&found="));
    t.app
        .get(&format!("{page}?result=not_found&found="))
        .await
        .assert_ok()
        .assert_see("No e-invoicing ID found for this TRN; enter it by hand");
    assert_eq!(stored(&t, &party).await, None);

    asp.say(Reply::Error);
    t.app
        .htmx()
        .post(&format!("{page}/lookup"), &[])
        .await
        .assert_redirect(&format!("{page}?result=failed&found="));
    assert_eq!(stored(&t, &party).await, None);
    let connection = wl_kernel::IntegrationConnection::by_key(t.db(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(connection.status, wl_kernel::ConnectionStatus::Up);
}

/// AC5: with the ASP connection not `up`, the lookup is disabled with the
/// banner, makes no call, and the identifier can still be entered by hand.
#[renox::test]
async fn ptyeinv06_with_the_connection_down_the_lookup_is_off_and_entry_by_hand_works() {
    let (t, asp) = setup().await;
    let party = company(&t, DELTA, Some(DELTA_TRN), None).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let page = format!("/parties/{}/einvoice", party.id);

    integrations::report_failure(t.app.state(), KEY, "timeout")
        .await
        .unwrap();
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("The lookup is not available now; enter the ID by hand");

    t.app
        .htmx()
        .post(&format!("{page}/lookup"), &[])
        .await
        .assert_redirect(&format!("{page}?result=unavailable&found="));
    assert_eq!(asp.calls(), 0);

    t.app
        .htmx()
        .post(&page, &[("einvoice_recipient_id", "0235:555")])
        .await
        .assert_redirect(&format!("/parties/{}", party.id));
    assert_eq!(stored(&t, &party).await.as_deref(), Some("0235:555"));
}

/// AC5, with the connection really `down`: no success for N minutes, the
/// scheduled `go_down` runs, the page shows the kernel banner with the
/// manual fallback, the Look up button is rendered disabled, a posted lookup
/// calls nothing, and the ID is still entered by hand.
#[renox::test]
async fn ptyeinv06_with_the_connection_marked_down_the_button_is_disabled_and_the_banner_shows() {
    let (t, asp) = setup().await;
    let party = company(&t, DELTA, Some(DELTA_TRN), None).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let page = format!("/parties/{}/einvoice", party.id);

    // Up: the button is live and there is no banner.
    let before = t.app.get(&page).await;
    before.assert_ok();
    assert!(!before.text().contains("Enter the e-invoicing ID by hand."));
    assert!(!before.text().contains("disabled"), "{}", before.text());

    integrations::report_failure(t.app.state(), KEY, "timeout")
        .await
        .unwrap();
    // Degraded is not down yet: no banner, and the button is already off.
    let degraded = t.app.get(&page).await;
    degraded.assert_ok();
    assert!(
        !degraded
            .text()
            .contains("Enter the e-invoicing ID by hand.")
    );
    assert!(degraded.text().contains("disabled"), "{}", degraded.text());

    // N minutes (default 15) without a success: the scheduled run takes it down.
    t.app.travel(std::time::Duration::from_secs(15 * 60));
    t.app
        .run_scheduled(integrations::service::TASK)
        .await
        .expect("the integration task runs");
    let connection = integrations::IntegrationConnection::by_key(t.db(), KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.status, integrations::ConnectionStatus::Down);

    let down = t.app.get(&page).await;
    down.assert_ok();
    let html = down.text();
    assert!(html.contains("Enter the e-invoicing ID by hand."), "{html}");
    assert!(html.contains("disabled"), "{html}");
    assert!(
        html.contains("The lookup is not available now; enter the ID by hand"),
        "{html}"
    );
    t.app
        .htmx()
        .post(&format!("{page}/lookup"), &[])
        .await
        .assert_redirect(&format!("{page}?result=unavailable&found="));
    assert_eq!(asp.calls(), 0);

    t.app
        .htmx()
        .post(&page, &[("einvoice_recipient_id", "0235:777")])
        .await
        .assert_redirect(&format!("/parties/{}", party.id));
    assert_eq!(stored(&t, &party).await.as_deref(), Some("0235:777"));
}

/// A company without a TRN cannot use the lookup: the button is off and the
/// request is refused.
#[renox::test]
async fn ptyeinv07_a_company_without_a_trn_cannot_look_up() {
    let (t, asp) = setup().await;
    let party = company(&t, "Sigma Trading LLC", None, None).await;
    let staff = t.user("Sami", &[perm::VIEW, perm::UPDATE]).await;
    t.app.acting_as(&staff);
    let page = format!("/parties/{}/einvoice", party.id);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("The lookup is not available now");
    t.app
        .htmx()
        .post(&format!("{page}/lookup"), &[])
        .await
        .assert_status(422);
    assert_eq!(asp.calls(), 0);
}

// BL-PTY-11 (#460): *Look up* in the company create form, by the TRN typed,
// before the party is saved.

/// The create form as typed: a company named Delta with `trn` and `id`.
fn typed<'a>(trn: &'a str, id: &'a str) -> Vec<(&'static str, &'a str)> {
    vec![
        ("kind", "company"),
        ("legal_name_en", DELTA),
        ("email", "accounts@delta.example"),
        ("default_language", "en"),
        ("preferred_channel", "email"),
        ("trn", trn),
        ("einvoice_recipient_id", id),
    ]
}

const CREATE_LOOKUP: &str = "/parties/new/einvoice-lookup";

async fn parties(t: &Booted) -> usize {
    Party::query().get(t.db()).await.unwrap().len()
}

/// Done when 1: with TRN 100123456700003 typed and the ASP returning
/// `0235:100123456700003`, *Look up* fills the field and keeps what was
/// typed; no party exists until *Save*, which stores the found ID.
#[renox::test]
async fn ptyeinv11_look_up_in_the_create_form_fills_the_field_and_saves_nothing() {
    let (t, asp) = setup().await;
    let staff = t.user("Sami", &[perm::VIEW, perm::CREATE]).await;
    t.app.acting_as(&staff);

    let form = t.app.get("/parties/new").await;
    form.assert_ok();
    assert!(form.text().contains(CREATE_LOOKUP), "{}", form.text());

    let found = "0235:100123456700003";
    let page = t.app.post(CREATE_LOOKUP, &typed(DELTA_TRN, "")).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains(&format!(r#"value="{found}""#)), "{html}");
    assert!(
        html.contains(r#"value="Delta Trading LLC""#),
        "what was typed is kept: {html}"
    );
    assert!(html.contains(r#"value="accounts@delta.example""#), "{html}");
    assert!(html.contains(&format!(r#"value="{DELTA_TRN}""#)), "{html}");
    assert_eq!(asp.calls(), 1);
    assert_eq!(parties(&t).await, 0, "no party until Save");

    let saved = t.app.post("/parties", &typed(DELTA_TRN, found)).await;
    assert!(saved.status.is_redirection(), "{}", saved.text());
    let party = t.party(DELTA).await;
    assert_eq!(stored(&t, &party).await.as_deref(), Some(found));
}

/// An ID the user typed is not replaced: the found one is offered, and
/// taken only when the user accepts it. Nothing is saved either way.
#[renox::test]
async fn ptyeinv11_a_typed_id_is_kept_and_the_found_one_offered() {
    let (t, asp) = setup().await;
    let staff = t.user("Sami", &[perm::VIEW, perm::CREATE]).await;
    t.app.acting_as(&staff);

    let page = t.app.post(CREATE_LOOKUP, &typed(DELTA_TRN, "0235:1")).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains(r#"value="0235:1""#), "{html}");
    assert!(html.contains("Use the found ID"), "{html}");
    assert!(html.contains("0235:100123456700003"), "{html}");
    assert_eq!(asp.calls(), 1);

    let mut accept = typed(DELTA_TRN, "0235:1");
    accept.push(("einvoice_suggestion", "0235:100123456700003"));
    accept.push(("accept_suggestion", "1"));
    let page = t.app.post(CREATE_LOOKUP, &accept).await;
    page.assert_ok();
    assert!(page.text().contains(r#"value="0235:100123456700003""#));
    assert_eq!(asp.calls(), 1, "accepting asks nothing");
    assert_eq!(parties(&t).await, 0);
}

/// Done when 2: with the connection `down`, the button in the form is
/// disabled with its reason, the banner shows, no ASP call is made, and
/// saving with an identifier typed by hand works.
#[renox::test]
async fn ptyeinv11_with_the_connection_down_the_form_lookup_is_off_and_saving_by_hand_works() {
    let (t, asp) = setup().await;
    let staff = t.user("Sami", &[perm::VIEW, perm::CREATE]).await;
    t.app.acting_as(&staff);

    integrations::report_failure(t.app.state(), KEY, "timeout")
        .await
        .unwrap();
    t.app.travel(std::time::Duration::from_secs(15 * 60));
    t.app
        .run_scheduled(integrations::service::TASK)
        .await
        .expect("the integration task runs");

    let form = t.app.get("/parties/new").await;
    form.assert_ok();
    let html = form.text();
    assert!(
        html.contains("Enter the e-invoicing ID by hand."),
        "the banner: {html}"
    );
    assert!(
        html.contains("The lookup is not available now; enter the ID by hand"),
        "the reason: {html}"
    );
    assert!(
        !html.contains(CREATE_LOOKUP),
        "no live Look up button: {html}"
    );

    let page = t.app.post(CREATE_LOOKUP, &typed(DELTA_TRN, "")).await;
    page.assert_ok();
    assert!(page.text().contains("The lookup is not available now"));
    assert_eq!(asp.calls(), 0);
    assert_eq!(parties(&t).await, 0);

    let saved = t.app.post("/parties", &typed(DELTA_TRN, "0235:555")).await;
    assert!(saved.status.is_redirection(), "{}", saved.text());
    let party = t.party(DELTA).await;
    assert_eq!(stored(&t, &party).await.as_deref(), Some("0235:555"));
    assert_eq!(asp.calls(), 0);
}

/// Done when 3: with no TRN typed, the button is disabled ("Enter the TRN
/// first"); a lookup sent anyway, or with a TRN that is not 15 digits, asks
/// nothing.
#[renox::test]
async fn ptyeinv11_with_no_trn_typed_the_button_is_disabled() {
    let (t, asp) = setup().await;
    let staff = t.user("Sami", &[perm::VIEW, perm::CREATE]).await;
    t.app.acting_as(&staff);

    let form = t.app.get("/parties/new").await;
    form.assert_ok();
    let html = form.text();
    assert!(html.contains("Enter the TRN first"), "{html}");
    assert!(html.contains("disabled"), "{html}");

    for trn in ["", "12345"] {
        let page = t.app.post(CREATE_LOOKUP, &typed(trn, "")).await;
        page.assert_ok();
        assert!(page.text().contains("Enter the TRN first"));
    }
    assert_eq!(asp.calls(), 0);
    assert_eq!(parties(&t).await, 0);
}
