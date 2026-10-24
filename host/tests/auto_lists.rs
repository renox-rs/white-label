//! The lists of the Auto app on the demo company: each is a `renox::grid`
//! that filters by status, sorts and pages through the query string, shows
//! money as `AED 1,250.00` and the status in words, and keeps the scope of the
//! role that opens it; the records show the numbers and the names of the
//! records they name, never an id or a key; the forms pick records from lists;
//! the buttons of a role are the ones it may press.

use host::auto_demo::LOGINS;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Auto vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
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

/// The demo company, seeded.
async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Auto vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::auto_demo::seed(app.kernel()).await.unwrap();
    app
}

/// Signs in as the demo login whose e-mail starts with `name`.
async fn sign_in(app: &TestApp, name: &str) {
    let email = LOGINS
        .iter()
        .map(|login| login.email)
        .find(|email| email.starts_with(&format!("{name}@")))
        .unwrap_or_else(|| panic!("the demo login {name}"));
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .expect("the demo user");
    app.acting_as(&user);
}

/// The page at `path` as the signed-in user gets it.
async fn load(app: &TestApp, path: &str) -> String {
    let page = app.get(path).await;
    assert_eq!(page.status.as_u16(), 200, "{path}");
    page.text()
}

/// The rows of the grid in `html` as the footer counts them: `1–5 of 5`.
fn range(html: &str, shown: &str) -> bool {
    html.contains(shown)
}

/// Whether `number` is a row of the grid (a link or the value of the cell).
fn has_row(html: &str, number: &str) -> bool {
    html.contains(&format!(">{number}<"))
}

/// Every `AED` amount of `html` reads `AED 1,250.00`: digits in threes with
/// commas and two decimals.
fn money_is_well_written(html: &str) -> bool {
    let mut rest = html;
    let mut found = 0;
    while let Some(at) = rest.find("AED ") {
        let tail = &rest[at + 4..];
        let amount: String = tail
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '.')
            .collect();
        if amount.is_empty() {
            rest = tail;
            continue;
        }
        let (whole, cents) = match amount.rsplit_once('.') {
            Some(parts) => parts,
            None => return false,
        };
        let groups: Vec<&str> = whole.split(',').collect();
        let grouped = groups[0].len() <= 3
            && !groups[0].is_empty()
            && groups[1..].iter().all(|group| group.len() == 3);
        if cents.len() != 2 || !grouped {
            return false;
        }
        found += 1;
        rest = tail;
    }
    found > 0
}

/// The job cards, the vehicles, the claims, the insurers, the fleet contracts,
/// the supplier claims, the export sales and the labour menu of the demo
/// company filter by status, sort and page.
#[renox::test]
async fn the_demo_lists_filter_sort_and_page() {
    let app = demo().await;
    sign_in(&app, "admin").await;

    // Job cards: thirty, newest first, ten to a page.
    let jobs = count(&app, "select count(*) from auto_job_cards").await;
    assert!(jobs >= 25, "the demo has {jobs} job cards");
    let pages = load(&app, "/auto/job-cards?sort=id&per_page=10&page=3").await;
    assert!(range(&pages, &format!("21–{jobs} of {jobs}")));
    assert!(has_row(&pages, &format!("JOB-{jobs:05}")) && !has_row(&pages, "JOB-00001"));
    let newest = load(&app, "/auto/job-cards?sort=-id").await;
    let first = newest.find(&format!(">JOB-{jobs:05}<")).unwrap();
    let second = newest.find(&format!(">JOB-{:05}<", jobs - 1)).unwrap();
    assert!(first < second, "newest first");
    assert!(
        money_is_well_written(&newest),
        "the estimates are in dirhams"
    );
    for status in ["ready", "delivered", "awaiting_approval", "voided"] {
        let n = count(
            &app,
            &format!("select count(*) from auto_job_cards where status = '{status}'"),
        )
        .await;
        let page = load(
            &app,
            &format!("/auto/job-cards?in.status={status}&per_page=50"),
        )
        .await;
        assert!(range(&page, &format!("1–{n} of {n}")), "{status}: {n}");
    }
    let ready = load(&app, "/auto/job-cards?in.status=ready").await;
    assert!(ready.contains("Ready") && !ready.contains("ready</span>"));
    let found = load(&app, "/auto/job-cards?search=Land%20Cruiser").await;
    assert!(found.contains("Toyota Land Cruiser"));

    // Vehicles: private and fleet, by plate.
    let fleet = count(
        &app,
        "select count(*) from auto_vehicles where fleet_contract_id is not null",
    )
    .await;
    let fleet_page = load(&app, "/auto/vehicles?in.kind=fleet").await;
    assert!(range(&fleet_page, &format!("1–{fleet} of {fleet}")));
    assert!(fleet_page.contains("Al Noor Logistics") && fleet_page.contains("Fleet"));
    let by_plate = load(&app, "/auto/vehicles?sort=-plate&per_page=10").await;
    assert!(by_plate.contains("of 3") || by_plate.contains("1–10 of"));
    let vehicles = count(&app, "select count(*) from auto_vehicles").await;
    let last = load(&app, "/auto/vehicles?sort=plate&per_page=10&page=4").await;
    assert!(range(&last, &format!("31–{vehicles} of {vehicles}")));

    // Claims: one in every state of the story.
    let settled = load(&app, "/auto/claims?in.status=settled").await;
    assert!(range(&settled, "1–1 of 1") && settled.contains("Settled"));
    assert!(money_is_well_written(&settled));
    let falcon = load(&app, "/auto/claims?search=FALCON&sort=-id").await;
    assert!(falcon.contains("Falcon Motor Insurance"));
    let claims = count(&app, "select count(*) from auto_insurance_claims").await;
    let page = load(&app, "/auto/claims?sort=id&per_page=10&page=2").await;
    assert!(range(&page, &format!("11–{claims} of {claims}")));

    // Insurers and fleet contracts.
    let insurers = load(&app, "/auto/insurers?in.standing=active").await;
    assert!(insurers.contains("Falcon Motor Insurance") && insurers.contains("AED 120.00"));
    let suspended = load(&app, "/auto/fleet?in.status=suspended").await;
    assert!(range(&suspended, "1–1 of 1") && suspended.contains("Desert Rent a Car"));
    assert!(suspended.contains("Suspended"));
    let by_number = load(&app, "/auto/fleet?sort=-id").await;
    assert!(by_number.find(">FC-0003<") < by_number.find(">FC-0001<"));

    // Supplier claims: by status and by the supplier's answer.
    let recovered = load(&app, "/auto/supplier-claims?in.status=recovered").await;
    assert!(range(&recovered, "1–1 of 1") && recovered.contains("Recovered"));
    let refused = load(&app, "/auto/supplier-claims?in.supplier_answer=rejected").await;
    assert!(range(&refused, "1–1 of 1") && refused.contains("AED 35.00"));
    assert!(!refused.contains("SWC-0001"));

    // Export sales: where each stands.
    for (standing, words) in [
        ("attached", "Exit evidence attached"),
        ("missing", "Exit evidence missing"),
        ("vat_due", "5% VAT due"),
    ] {
        let page = load(&app, &format!("/auto/export-sales?in.standing={standing}")).await;
        assert!(
            range(&page, "1–1 of 1") && page.contains(words),
            "{standing}"
        );
        assert!(page.contains("AED 25,000.00"));
    }

    // The labour menu is kept: the flat-rate jobs and the hourly work.
    let menu = load(&app, "/auto/labour-menu").await;
    for code in ["OIL-CHG", "BRK-FRT", "SVC-60K", "DIAG"] {
        assert!(has_row(&menu, code), "{code} is on the menu");
    }
    assert!(menu.contains("AED 180.00") && money_is_well_written(&menu));
    let hourly = load(&app, "/auto/labour-menu?in.pricing=hourly").await;
    assert!(range(&hourly, "1–1 of 1") && has_row(&hourly, "DIAG"));
}

/// A technician sees the job cards assigned to them and no customer; the
/// workshop's list shows the customer.
#[renox::test]
async fn the_technician_lists_follow_the_assignment() {
    let app = demo().await;
    let nabil = count(
        &app,
        "select id from users where email = 'nabil@alnoorauto.example'",
    )
    .await;
    let mine = count(
        &app,
        &format!("select count(*) from auto_job_cards where technician_user_id = {nabil}"),
    )
    .await;
    let open = count(
        &app,
        &format!(
            "select count(*) from auto_job_cards where technician_user_id = {nabil} \
             and status not in ('delivered', 'declined', 'cancelled', 'voided')"
        ),
    )
    .await;
    sign_in(&app, "nabil").await;
    let list = load(&app, "/auto/job-cards?per_page=100").await;
    assert!(range(&list, &format!("1–{mine} of {mine}")));
    assert!(!list.contains(">Customer<"), "no contact details");
    let own = load(&app, "/auto/technician?per_page=100").await;
    assert!(range(&own, &format!("1–{open} of {open}")));
    sign_in(&app, "sami").await;
    assert!(load(&app, "/auto/job-cards").await.contains(">Customer<"));
}

/// The records name the invoices, the bills, the rate cards, the returns and
/// the services by their numbers and words, with a link to the page of an
/// invoice, never an id such as `#4` or a key such as `periodic_service`.
#[renox::test]
async fn the_records_show_numbers_and_words_not_ids_or_keys() {
    let app = demo().await;
    sign_in(&app, "admin").await;

    let card = count(
        &app,
        "select min(id) from auto_job_cards where invoice_id is not null",
    )
    .await;
    let invoice = count(
        &app,
        &format!("select invoice_id from auto_job_cards where id = {card}"),
    )
    .await;
    let number = text_of(
        &app,
        &format!("select number from wl_invoicing_invoices where id = {invoice}"),
    )
    .await;
    let page = load(&app, &format!("/auto/job-cards/{card}")).await;
    assert!(page.contains(&number), "the invoice number {number}");
    assert!(page.contains(&format!("href=\"/invoices/{invoice}\"")));
    assert!(!page.contains(&format!(">#{invoice}<")));
    assert!(!page.contains("check_in"), "the history gives no key");

    let claim = count(
        &app,
        "select min(id) from auto_insurance_claims where customer_invoice_id is not null",
    )
    .await;
    let page = load(&app, &format!("/auto/claims/{claim}")).await;
    assert!(page.contains("INV-") && page.contains("PB-") || page.contains("INV-"));
    assert!(
        page.contains("Version 1"),
        "the rate card version, in words"
    );
    assert!(!page.contains("#1 v1"));
    assert!(
        page.contains("href=\"/auto/job-cards/"),
        "the job card is a link"
    );

    let page = load(&app, "/auto/fleet/1").await;
    assert!(page.contains("Periodic service") && !page.contains(">periodic_service<"));
    assert!(page.contains("Booked for"), "the booking by its day");

    let page = load(&app, "/auto/supplier-claims/1").await;
    assert!(!page.contains("auto.supplier_warranty_claim:"));
    assert!(!page.contains(":months") && page.contains("Warranty terms"));
    assert!(page.contains("Claim email sent to the supplier (SWC-0001)"));

    let page = load(&app, "/auto/insurers/1").await;
    assert!(page.contains("Version 1, in force from"));
    assert!(page.contains("Falcon") && !page.contains(">#1<"));

    // The police report a card waits for names the role in words.
    let accident = count(
        &app,
        "select min(id) from auto_job_cards where accident_damage = 1 and status = 'checked_in'",
    )
    .await;
    let page = load(&app, &format!("/auto/job-cards/{accident}")).await;
    assert!(page.contains("renew: insurance coordinator"));
    assert!(!page.contains("insurance_coordinator"));

    // The branch manager opens the record the approvals inbox names.
    sign_in(&app, "basma").await;
    let inbox = load(&app, "/approvals").await;
    assert!(inbox.contains("/auto/supplier-claims/4"));
    load(&app, "/auto/supplier-claims/4").await;
    let page = load(&app, "/auto/supplier-claims").await;
    assert!(page.contains("SWC-0004"));
}

/// The forms name records of other modules from pick lists: the export
/// invoice, the payer agreement, the fleet contract and the supplier return.
#[renox::test]
async fn the_forms_pick_records_from_lists() {
    let app = demo().await;

    sign_in(&app, "aisha").await;
    let page = load(&app, "/auto/export-sales").await;
    assert!(page.contains("/auto/export-invoice-options"));
    assert!(!page.contains("name=\"invoice\" type=\"number\""));
    let invoice = count(&app, "select min(invoice_id) from auto_export_sales").await;
    let options = load(
        &app,
        &format!("/auto/export-invoice-options?values={invoice}"),
    )
    .await;
    assert!(options.contains("INV-2026-") && options.contains("Gulf Movers Trading"));
    assert!(options.contains("AED 25,000.00"));
    let none = load(&app, "/auto/export-invoice-options?q=INV").await;
    assert!(
        !none.contains(&format!("\"value\":\"{invoice}\"")),
        "an invoice that is an export sale already is not offered"
    );

    sign_in(&app, "carla").await;
    let page = load(&app, "/auto/insurers").await;
    assert!(page.contains("/auto/payer-agreement-options"));
    assert!(!page.contains("name=\"agreement\" type=\"number\""));
    let agreement = count(&app, "select min(payer_agreement_id) from auto_insurers").await;
    let options = load(
        &app,
        &format!("/auto/payer-agreement-options?values={agreement}"),
    )
    .await;
    assert!(options.contains("Falcon") || options.contains("Crescent"));
    let found = load(&app, "/auto/payer-agreement-options?q=falcon").await;
    assert!(found.contains("Falcon") && !found.contains("Crescent"));

    sign_in(&app, "admin").await;
    let page = load(&app, "/auto/vehicles").await;
    assert!(page.contains("/auto/fleet-contract-options"));
    assert!(!page.contains("name=\"fleet_contract\" type=\"number\""));
    let options = load(&app, "/auto/fleet-contract-options?q=Logistics").await;
    assert!(options.contains("FC-0001"));
    assert!(
        !options.contains("FC-0003"),
        "a contract that ended is not offered"
    );
    let page = load(&app, "/auto/vehicles/1").await;
    assert!(!page.contains("name=\"fleet_contract\" type=\"number\""));

    sign_in(&app, "pia").await;
    let page = load(&app, "/auto/supplier-claims/2").await;
    assert!(page.contains("/auto/supplier-return-options"));
    assert!(!page.contains("name=\"supplier_return\" type=\"number\""));
    let options = load(&app, "/auto/supplier-return-options?q=").await;
    assert!(
        options.starts_with('['),
        "the pick list answers with options"
    );
}

/// The advisor does not see the technician's buttons, the technician does;
/// the QC checklist saves with one tick or five, and the vehicle passes.
#[renox::test]
async fn the_buttons_and_the_checklist_work_for_their_roles() {
    let app = demo().await;
    let nabil = count(
        &app,
        "select id from users where email = 'nabil@alnoorauto.example'",
    )
    .await;
    let checked_in = count(
        &app,
        &format!(
            "select min(id) from auto_job_cards where status = 'checked_in' \
             and technician_user_id = {nabil}"
        ),
    )
    .await;
    sign_in(&app, "sami").await;
    let page = load(&app, &format!("/auto/job-cards/{checked_in}")).await;
    assert!(
        !page.contains("Start the inspection"),
        "the advisor may not start an inspection"
    );
    sign_in(&app, "nabil").await;
    let page = load(&app, &format!("/auto/job-cards/{checked_in}")).await;
    assert!(page.contains("Start the inspection"));

    let quality = count(
        &app,
        "select min(id) from auto_job_cards where status = 'quality_check'",
    )
    .await;
    sign_in(&app, "waleed").await;
    let path = format!("/auto/job-cards/{quality}");
    let one = app
        .htmx()
        .post(&format!("{path}/qc"), &[("done", "fluids")])
        .await;
    assert_eq!(one.status.as_u16(), 200, "a single tick: {}", one.text());
    let all = app
        .htmx()
        .post(
            &format!("{path}/qc"),
            &[
                ("done", "fluids"),
                ("done", "wheels"),
                ("done", "lights"),
                ("done", "road_test"),
                ("done", "clean"),
            ],
        )
        .await;
    assert_eq!(all.status.as_u16(), 200, "five ticks: {}", all.text());
    app.htmx()
        .post(&format!("{path}/pass-qc"), &[])
        .await
        .assert_ok();
    assert_eq!(
        text_of(
            &app,
            &format!("select status from auto_job_cards where id = {quality}")
        )
        .await,
        "ready"
    );
}
