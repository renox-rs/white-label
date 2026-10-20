//! BL-PRP-26 (#276) item 6: e-invoicing under the Property client. The
//! go-live date is the client's (the host leaves it; `app_for_tuned` sets it
//! the way a client does), the rest is the Property host with its real
//! modules: with go-live 2027-07-01 a commercial-lease invoice to a company
//! issued 2027-07-02 is transmitted to the ASP, one issued 2027-06-30 is not,
//! a residential invoice to an individual (B2C) is never transmitted, and with
//! the ASP down the commercial invoice is queued.

mod support;

use host::roles::RECEIVABLES;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::einvoice_adapter::FakeAsp;
use wl_invoicing::{EInvoiceTransmission, TransmissionStatus};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::{ClientConfig, PartyId, PartyKind};
use wl_parties::{CompanyDetails, ContactPoint, ContactType, NewParty, create};

const PEPPOL: &str = "0235:100000000000011";

fn go_live() -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, 7, 1).unwrap()
}

/// The Property host whose entity PGP is e-invoicing from 2027-07-01 through
/// the scripted `asp`.
async fn booted_with(asp: &FakeAsp) -> TestApp {
    let asp = asp.clone();
    let config = ClientConfig::load(host::DEMO_CLIENT).expect("the demo client");
    let tuning = host::Tuning::default().invoicing(move |invoicing| {
        invoicing
            .einvoicing("PGP", go_live(), "asp")
            .asp_adapter("asp", asp)
    });
    let app = host::app_for_tuned(config, tuning).expect("the app");
    let app = TestApp::new(app.provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Moves the clock to `date` 06:00 UTC (10:00 in Dubai).
fn travel_to(app: &TestApp, date: &str) {
    let target = date
        .parse::<NaiveDate>()
        .expect("a date")
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc();
    app.travel_back();
    let seconds = (target - renox::db::now()).num_seconds();
    assert!(seconds > 0, "{target} is in the past");
    app.travel(std::time::Duration::from_secs(seconds as u64));
}

async fn transmissions(app: &TestApp) -> Vec<EInvoiceTransmission> {
    EInvoiceTransmission::query()
        .order_by(EInvoiceTransmission::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// A company with a TRN and the e-invoicing recipient identifier.
async fn shop(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Company,
            legal_name_en: "Palm Retail LLC".into(),
            company: Some(CompanyDetails {
                trn: Some("100000000000011".into()),
                vat_registered: true,
                einvoice_recipient_id: Some(PEPPOL.into()),
                ..Default::default()
            }),
            contacts: vec![ContactPoint {
                contact_type: ContactType::Email,
                value: "accounts@palmretail.example".into(),
                is_primary: true,
                ..Default::default()
            }],
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

#[renox::test]
async fn the_commercial_invoice_is_transmitted_from_go_live_and_the_residential_one_never() {
    let asp = FakeAsp::new();
    let app = booted_with(&asp).await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let shop = shop(&app).await;
    let tara = tenant(&app, "Tara Nasser").await;

    // The day before go-live: issued, not transmitted.
    travel_to(&app, "2027-06-30");
    let early = issued_invoice(&app, &clerk, shop, 100_000, VatCategory::Standard, day(0)).await;
    assert!(early.number.is_some());
    assert!(transmissions(&app).await.is_empty());
    assert!(asp.submitted().is_empty());

    // The day after: the company's invoice goes to the ASP, the individual's
    // exempt residential rent does not.
    travel_to(&app, "2027-07-02");
    let late = issued_invoice(&app, &clerk, shop, 100_000, VatCategory::Standard, day(0)).await;
    let flat = issued_invoice(&app, &clerk, tara, 100_000, VatCategory::Exempt, day(0)).await;
    assert!(flat.number.is_some(), "the B2C invoice is issued");
    let sent = transmissions(&app).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].document(), late.subject());
    assert_eq!(sent[0].status, TransmissionStatus::Submitted);
    assert_eq!(sent[0].recipient_id.as_deref(), Some(PEPPOL));
    assert_eq!(asp.submitted().len(), 1);

    // The ASP down: the next commercial invoice is queued, not lost.
    asp.unreachable(true);
    let queued = issued_invoice(&app, &clerk, shop, 100_000, VatCategory::Standard, day(0)).await;
    let all = transmissions(&app).await;
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].document(), queued.subject());
    assert_eq!(all[1].status, TransmissionStatus::Queued);
    assert_eq!(asp.submitted().len(), 1, "nothing more reached the ASP");
}
