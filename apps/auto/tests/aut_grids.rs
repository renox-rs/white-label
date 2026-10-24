//! The lists of the app are `renox::grid`s: each loads with the workshop's
//! records, filters by status (or by its other select column), sorts by a
//! column, pages through the query string (`?in.status=`, `?sort=`,
//! `?per_page=&page=`), searches, shows its money as `AED 1,250.00` and its
//! status in words, and keeps the scope of the role that opens it.

mod support;

use auto::job_card::JobCard;
use auto::vehicles::{self, NewVehicle};
use auto::{
    ClaimStatus, Emirate, ExportEvidence, ExportSale, FleetContract, FleetStatus, InsuranceClaim,
    Insurer, JobCardStatus, LabourOperation, LabourPricing, SupplierAnswer, SupplierClaim,
    SupplierClaimStatus, Vehicle, permissions,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::claimfx::orient;
use support::{Garage, boot, day};

/// The page at `path`, as the signed-in user gets it.
async fn load(t: &Garage, path: &str) -> String {
    let page = t.app.get(path).await;
    page.assert_ok();
    page.text()
}

/// Whether `number` is a row of the grid (a link to its record).
fn has_row(html: &str, number: &str) -> bool {
    html.contains(&format!(">{number}<"))
}

/// Where the row `number` stands in the page; a row that is not there is last.
fn at(html: &str, number: &str) -> usize {
    html.find(&format!(">{number}<")).unwrap_or(usize::MAX)
}

async fn sign_in(t: &Garage, name: &str, grants: &[&str]) {
    let user = t.user(name, grants).await;
    t.app.acting_as(&user);
}

/// Twelve job cards of the Hilux: seven inspecting, five declined; the first
/// has an approved labour line of AED 300 (AED 315.00 with VAT).
async fn twelve_cards(t: &Garage) -> Vec<JobCard> {
    let vehicle = t.hilux().await;
    let mut cards = Vec::new();
    for n in 0..12 {
        let status = if n < 7 {
            JobCardStatus::Inspecting
        } else {
            JobCardStatus::Declined
        };
        cards.push(t.card_in(&vehicle, status).await);
    }
    t.approved_labour(&cards[0], "Brake service", 2_000, 15_000)
        .await;
    cards
}

/// The job cards grid filters by status, sorts, pages and searches; money is
/// in dirhams and the status in words.
#[renox::test]
async fn aut_grid_the_job_cards_filter_sort_and_page() {
    let t = boot().await;
    let cards = twelve_cards(&t).await;
    sign_in(&t, "Sami", &[permissions::JOB_CARDS_VIEW]).await;
    let n = |i: usize| cards[i].number();

    let all = load(&t, "/auto/job-cards?sort=-id&per_page=10").await;
    assert!(all.contains("1–10 of 12"));
    assert!(has_row(&all, &n(11)) && has_row(&all, &n(2)));
    assert!(!has_row(&all, &n(1)) && !has_row(&all, &n(0)));
    assert!(at(&all, &n(11)) < at(&all, &n(10)), "newest first");
    let second = load(&t, "/auto/job-cards?sort=-id&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12"));
    assert!(has_row(&second, &n(1)) && has_row(&second, &n(0)));
    assert!(!has_row(&second, &n(11)));

    let oldest = load(&t, "/auto/job-cards?sort=id").await;
    assert!(at(&oldest, &n(0)) < at(&oldest, &n(1)), "oldest first");

    let declined = load(&t, "/auto/job-cards?in.status=declined").await;
    assert!(declined.contains("1–5 of 5"));
    assert!(has_row(&declined, &n(11)) && has_row(&declined, &n(7)));
    assert!(!has_row(&declined, &n(6)) && !has_row(&declined, &n(0)));
    assert!(declined.contains("Declined") && !declined.contains("declined</span>"));
    let two = load(
        &t,
        "/auto/job-cards?in.status=declined&in.status=inspecting&per_page=50",
    )
    .await;
    assert!(two.contains("1–12 of 12"));

    let money = load(&t, "/auto/job-cards?in.status=inspecting").await;
    assert!(money.contains("AED 315.00"), "the estimate with VAT");
    assert!(money.contains("Inspecting") && money.contains("Mona Hassan"));

    let found = load(&t, "/auto/job-cards?search=hilux").await;
    assert!(found.contains("1–10 of 12") || found.contains("of 12"));
    let none = load(&t, "/auto/job-cards?search=ferrari").await;
    assert!(!has_row(&none, &n(0)));
}

/// A technician sees only the job cards assigned to them, with no customer
/// column; `/auto/technician` lists their open ones.
#[renox::test]
async fn aut_grid_a_technician_sees_only_their_job_cards() {
    let t = boot().await;
    let cards = twelve_cards(&t).await;
    let nabil = t.technician_user("Nabil").await;
    JobCard::query()
        .where_(JobCard::ID.eq(cards[0].id))
        .update(t.db(), &[("technician_user_id", &Some(nabil.id))])
        .await
        .unwrap();
    let tareq = t.technician_user("Tareq").await;
    t.app.acting_as(&tareq);

    let list = load(&t, "/auto/job-cards?per_page=50").await;
    assert!(has_row(&list, &cards[1].number()) && has_row(&list, &cards[9].number()));
    assert!(
        !has_row(&list, &cards[0].number()),
        "another technician's card"
    );
    assert!(!list.contains("Mona Hassan"), "no contact details");
    assert!(!list.contains(">Customer<"));

    let mine = load(&t, "/auto/technician?per_page=50").await;
    assert!(has_row(&mine, &cards[1].number()));
    assert!(!has_row(&mine, &cards[0].number()));
    assert!(
        !has_row(&mine, &cards[9].number()),
        "a declined card is not open"
    );
    assert!(mine.contains("1–6 of 6"));
}

/// The vehicles grid filters by kind, sorts by plate, pages and searches.
#[renox::test]
async fn aut_grid_the_vehicles_filter_sort_and_page() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = t.advisor().await;
    let mut ids = Vec::new();
    for n in 0..12 {
        let vehicle = vehicles::create(
            t.state(),
            &sami,
            NewVehicle::new(
                Emirate::Dubai,
                "B",
                &format!("{}", 10_001 + n),
                "Toyota",
                "Hilux",
                7,
            )
            .odometer(1_000 * (n + 1)),
        )
        .await
        .unwrap();
        ids.push(vehicle.id);
    }
    for id in &ids[..3] {
        Vehicle::query()
            .where_(Vehicle::ID.eq(*id))
            .update(t.db(), &[("fleet_contract_id", &Some(1_i64))])
            .await
            .unwrap();
    }
    sign_in(&t, "Nadia", &[permissions::VEHICLES_VIEW]).await;

    let fleet = load(&t, "/auto/vehicles?in.kind=fleet").await;
    assert!(fleet.contains("1–3 of 3"));
    assert!(has_row(&fleet, "Dubai B 10001") && !has_row(&fleet, "Dubai B 10004"));
    assert!(fleet.contains("Fleet"));
    let private = load(&t, "/auto/vehicles?in.kind=private&per_page=10").await;
    assert!(private.contains("1–9 of 9"));

    let desc = load(&t, "/auto/vehicles?sort=-plate&per_page=10").await;
    assert!(desc.contains("1–10 of 12"));
    assert!(at(&desc, "Dubai B 10012") < at(&desc, "Dubai B 10011"));
    assert!(!has_row(&desc, "Dubai B 10001"));
    let second = load(&t, "/auto/vehicles?sort=-plate&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && has_row(&second, "Dubai B 10001"));

    let found = load(&t, "/auto/vehicles?search=10007").await;
    assert!(has_row(&found, "Dubai B 10007") && !has_row(&found, "Dubai B 10008"));
    assert!(found.contains("Mona Hassan"), "the owner by name");
}

/// Twelve claims of Orient on the Hilux: eight registered, four voided.
async fn twelve_claims(t: &Garage) -> Vec<InsuranceClaim> {
    let insurer = orient(t).await;
    let vehicle = t.hilux().await;
    let mut claims = Vec::new();
    for n in 0..12 {
        let status = if n < 8 {
            ClaimStatus::Registered
        } else {
            ClaimStatus::Voided
        };
        let mut claim = InsuranceClaim {
            insurer_id: insurer.id,
            customer_party_id: 7,
            vehicle_id: vehicle.id,
            policy_no: format!("POL-{n:04}"),
            claim_no: format!("OR-2026-{n:03}"),
            accident_date: day(-10),
            approved_amount: if n == 0 { 125_000 } else { 0 },
            status,
            ..InsuranceClaim::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        claim
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        claims.push(claim);
    }
    claims
}

/// The claims grid filters by status, sorts, pages and searches by the claim
/// or the policy number.
#[renox::test]
async fn aut_grid_the_claims_filter_sort_and_page() {
    let t = boot().await;
    let claims = twelve_claims(&t).await;
    sign_in(&t, "Carla", &[permissions::INSURANCE_MANAGE]).await;
    let n = |i: usize| claims[i].number();

    let page = load(&t, "/auto/claims?sort=-id&per_page=10").await;
    assert!(page.contains("1–10 of 12") && has_row(&page, &n(11)) && !has_row(&page, &n(0)));
    assert!(at(&page, &n(11)) < at(&page, &n(10)));
    assert!(page.contains("Orient Insurance") && page.contains("Dubai A 12345"));
    let second = load(&t, "/auto/claims?sort=-id&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && has_row(&second, &n(0)));

    let voided = load(&t, "/auto/claims?in.status=voided").await;
    assert!(voided.contains("1–4 of 4") && has_row(&voided, &n(8)) && !has_row(&voided, &n(7)));
    assert!(voided.contains("Voided"));
    let registered = load(&t, "/auto/claims?in.status=registered&sort=id").await;
    assert!(registered.contains("1–8 of 8") && at(&registered, &n(0)) < at(&registered, &n(1)));
    assert!(
        registered.contains("AED 1,250.00"),
        "what the insurer approved"
    );

    let by_policy = load(&t, "/auto/claims?search=POL-0003").await;
    assert!(has_row(&by_policy, &n(3)) && !has_row(&by_policy, &n(4)));
    let by_claim = load(&t, "/auto/claims?search=OR-2026-005").await;
    assert!(has_row(&by_claim, &n(5)) && !has_row(&by_claim, &n(6)));
}

/// Twelve insurers: eight take claims, four stopped.
async fn twelve_insurers(t: &Garage) -> Vec<Insurer> {
    let mut insurers = Vec::new();
    for n in 0..12 {
        t.people.add(100 + n, &format!("Insurer {n:02}"));
        let mut insurer = Insurer {
            party_id: 100 + n,
            claim_email: Some(format!("claims{n}@insurer.example")),
            payment_terms_days: 30 + n,
            labour_rate: Some(12_000),
            active: n < 8,
            ..Insurer::default()
        };
        insurer.save(t.db()).await.unwrap();
        insurers.push(insurer);
    }
    insurers
}

/// The insurers grid filters by whether they take claims, sorts and pages.
#[renox::test]
async fn aut_grid_the_insurers_filter_sort_and_page() {
    let t = boot().await;
    twelve_insurers(&t).await;
    sign_in(&t, "Carla", &[permissions::INSURANCE_MANAGE]).await;

    let page = load(&t, "/auto/insurers?sort=-id&per_page=10").await;
    assert!(page.contains("1–10 of 12"));
    assert!(page.contains("Insurer 11") && !page.contains("Insurer 00"));
    assert!(page.contains("AED 120.00"), "the labour rate in dirhams");
    let second = load(&t, "/auto/insurers?sort=-id&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && second.contains("Insurer 00"));

    let stopped = load(&t, "/auto/insurers?in.standing=stopped").await;
    assert!(stopped.contains("1–4 of 4"));
    assert!(stopped.contains("Insurer 08") && !stopped.contains("Insurer 07"));
    assert!(stopped.contains("Stopped") || stopped.contains("stopped"));
    let sorted = load(&t, "/auto/insurers?sort=payment_terms_days").await;
    assert!(sorted.find("Insurer 00") < sorted.find("Insurer 01"));
    let found = load(&t, "/auto/insurers?search=claims5@").await;
    assert!(found.contains("Insurer 05") && !found.contains("Insurer 06"));
}

/// A name for each of twelve records.
const NAMES: [&str; 12] = [
    "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliet",
    "Kilo", "Lima",
];

/// Twelve fleet contracts: eight drafts, four ended.
async fn twelve_contracts(t: &Garage) -> Vec<FleetContract> {
    t.people.add(20, "Al Noor Logistics");
    let mut contracts = Vec::new();
    for n in 0..12 {
        let status = if n < 8 {
            FleetStatus::Draft
        } else {
            FleetStatus::Ended
        };
        let mut contract = FleetContract {
            fleet_party_id: 20,
            title: format!("Depot {}", NAMES[n as usize]),
            start_date: day(-100),
            end_date: day(200),
            labour_rate: 12_000,
            notice_period_days: 30,
            renewal_window_days: 60,
            status,
            ..FleetContract::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        contract
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        contracts.push(contract);
    }
    contracts
}

/// The fleet contracts grid filters by status, sorts, pages, counts the
/// vehicles under contract and searches by number or name.
#[renox::test]
async fn aut_grid_the_fleet_contracts_filter_sort_and_page() {
    let t = boot().await;
    let contracts = twelve_contracts(&t).await;
    t.people.add(7, "Mona Hassan");
    let sami = t.advisor().await;
    let van = vehicles::create(
        t.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "B", "555", "Nissan", "Urvan", 20).odometer(1_000),
    )
    .await
    .unwrap();
    Vehicle::query()
        .where_(Vehicle::ID.eq(van.id))
        .update(t.db(), &[("fleet_contract_id", &Some(contracts[0].id))])
        .await
        .unwrap();
    sign_in(&t, "Fatima", &[permissions::FLEET_CONTRACTS_MANAGE]).await;
    let n = |i: usize| contracts[i].number();

    let page = load(&t, "/auto/fleet?sort=-id&per_page=10").await;
    assert!(page.contains("1–10 of 12") && has_row(&page, &n(11)) && !has_row(&page, &n(1)));
    assert!(page.contains("Al Noor Logistics") && page.contains("AED 120.00"));
    let second = load(&t, "/auto/fleet?sort=-id&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && has_row(&second, &n(0)));

    let ended = load(&t, "/auto/fleet?in.status=ended").await;
    assert!(ended.contains("1–4 of 4") && has_row(&ended, &n(8)) && !has_row(&ended, &n(7)));
    assert!(ended.contains("Ended"));
    let drafts = load(&t, "/auto/fleet?in.status=draft&sort=id").await;
    assert!(drafts.contains("1–8 of 8") && at(&drafts, &n(0)) < at(&drafts, &n(1)));
    let by_name = load(&t, "/auto/fleet?search=Lima").await;
    assert!(has_row(&by_name, &n(11)) && !has_row(&by_name, &n(10)));
    let counted = load(&t, &format!("/auto/fleet?search={}", n(0))).await;
    assert!(has_row(&counted, &n(0)));
    let cell = counted
        .find("<td data-col=\"vehicles\"")
        .expect("the vehicles column");
    assert!(
        counted[cell..cell + 160].contains(">1<"),
        "one vehicle under the contract: {}",
        &counted[cell..cell + 160]
    );
    let by_number = load(&t, &format!("/auto/fleet?search={}", n(5))).await;
    assert!(has_row(&by_number, &n(5)) && !has_row(&by_number, &n(6)));
}

/// Twelve supplier claims: eight raised, four recovered.
async fn twelve_supplier_claims(t: &Garage) -> Vec<SupplierClaim> {
    t.people.add(40, "Gulf Auto Spares");
    let mut claims = Vec::new();
    for n in 0..12 {
        let status = if n < 8 {
            SupplierClaimStatus::Raised
        } else {
            SupplierClaimStatus::Recovered
        };
        let mut claim = SupplierClaim {
            ticket_id: 500 + n,
            ticket_number: format!("TKT-2026-{n:04}"),
            supplier_party_id: 40,
            part_description: format!("Alternator {n:02}"),
            quantity_milli: 1_000,
            part_cost: 45_000,
            claim_amount: 45_000,
            purchase_date: day(-100),
            warranty_months: 12,
            failure_date: day(-5),
            supplier_answer: if n < 8 {
                SupplierAnswer::Pending
            } else {
                SupplierAnswer::Accepted
            },
            status,
            ..SupplierClaim::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        claim
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        claims.push(claim);
    }
    claims
}

/// The supplier claims grid filters by status and by the supplier's answer,
/// sorts, pages and searches by number, ticket or part.
#[renox::test]
async fn aut_grid_the_supplier_claims_filter_sort_and_page() {
    let t = boot().await;
    let claims = twelve_supplier_claims(&t).await;
    sign_in(&t, "Pia", &[permissions::SUPPLIER_CLAIMS_MANAGE]).await;
    let n = |i: usize| claims[i].number();

    let page = load(&t, "/auto/supplier-claims?sort=-id&per_page=10").await;
    assert!(page.contains("1–10 of 12") && has_row(&page, &n(11)) && !has_row(&page, &n(1)));
    assert!(page.contains("Gulf Auto Spares") && page.contains("AED 450.00"));
    let second = load(&t, "/auto/supplier-claims?sort=-id&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && has_row(&second, &n(0)));

    let recovered = load(&t, "/auto/supplier-claims?in.status=recovered").await;
    assert!(
        recovered.contains("1–4 of 4") && has_row(&recovered, &n(8)) && !has_row(&recovered, &n(7))
    );
    assert!(recovered.contains("Recovered"));
    let accepted = load(&t, "/auto/supplier-claims?in.supplier_answer=accepted").await;
    assert!(accepted.contains("1–4 of 4"));
    let raised = load(&t, "/auto/supplier-claims?in.status=raised&sort=id").await;
    assert!(raised.contains("1–8 of 8") && at(&raised, &n(0)) < at(&raised, &n(1)));
    let by_ticket = load(&t, "/auto/supplier-claims?search=TKT-2026-0003").await;
    assert!(has_row(&by_ticket, &n(3)) && !has_row(&by_ticket, &n(4)));
    let by_part = load(&t, "/auto/supplier-claims?search=Alternator%2005").await;
    assert!(has_row(&by_part, &n(5)) && !has_row(&by_part, &n(6)));

    // The branch manager who decides the write-off opens the list too.
    sign_in(&t, "Basma", &[permissions::SUPPLIER_CLAIMS_APPROVE]).await;
    load(&t, "/auto/supplier-claims").await;
    load(&t, &format!("/auto/supplier-claims/{}", claims[0].id)).await;
}

/// Twelve export sales: waiting, with the evidence attached for the first
/// four.
async fn twelve_export_sales(t: &Garage) -> Vec<ExportSale> {
    t.people.add(60, "Gulf Movers Trading");
    let mut sales = Vec::new();
    for n in 0..12 {
        let mut sale = ExportSale {
            invoice_id: 900 + n,
            invoice_number: Some(format!("INV-2026-{n:04}")),
            party_id: 60,
            currency: "AED".to_owned(),
            gross_total: 2_500_000,
            supply_date: day(-(10 + n)),
            ..ExportSale::default()
        };
        sale.save(t.db()).await.unwrap();
        if n < 4 {
            ExportEvidence {
                export_sale_id: sale.id,
                kind: "bill_of_lading".to_owned(),
                reference: format!("BL-{n}"),
                ..ExportEvidence::default()
            }
            .save(t.db())
            .await
            .unwrap();
        }
        sales.push(sale);
    }
    sales
}

/// The export sales grid filters by where each stands, sorts by the supply
/// date, pages and searches by invoice number; the total is in dirhams.
#[renox::test]
async fn aut_grid_the_export_sales_filter_sort_and_page() {
    let t = boot().await;
    let sales = twelve_export_sales(&t).await;
    sign_in(&t, "Aisha", &[permissions::EXPORT_SALES_VIEW]).await;
    let inv = |i: usize| sales[i].invoice_number.clone().unwrap();

    let page = load(&t, "/auto/export-sales?sort=supply_date&per_page=10").await;
    assert!(page.contains("1–10 of 12"));
    assert!(
        at(&page, &inv(11)) < at(&page, &inv(10)),
        "the oldest supply first"
    );
    assert!(page.contains("Gulf Movers Trading") && page.contains("AED 25,000.00"));
    let second = load(&t, "/auto/export-sales?sort=supply_date&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && has_row(&second, &inv(1)));

    let attached = load(&t, "/auto/export-sales?in.standing=attached").await;
    assert!(attached.contains("1–4 of 4") && has_row(&attached, &inv(0)));
    assert!(!has_row(&attached, &inv(4)));
    assert!(
        attached.contains("Exit evidence attached") && attached.contains("Bill of lading BL-0")
    );
    let waiting = load(&t, "/auto/export-sales?in.standing=waiting").await;
    assert!(waiting.contains("1–8 of 8") && has_row(&waiting, &inv(4)));
    assert!(waiting.contains("Waiting for exit evidence"));
    let found = load(&t, "/auto/export-sales?search=INV-2026-0007").await;
    assert!(has_row(&found, &inv(7)) && !has_row(&found, &inv(8)));
}

/// Twelve labour operations: eight flat-rate on the menu, four hourly off it.
async fn twelve_operations(t: &Garage) -> Vec<LabourOperation> {
    let mut operations = Vec::new();
    for n in 0..12 {
        let mut operation = LabourOperation {
            code: format!("OP-{n:02}"),
            name_en: format!("Operation {n:02}"),
            pricing: if n < 8 {
                LabourPricing::FlatRate
            } else {
                LabourPricing::Hourly
            },
            hours_milli: if n < 8 { 1_500 } else { 0 },
            rate: 18_000,
            active: n < 8,
            ..LabourOperation::default()
        };
        operation.save(t.db()).await.unwrap();
        operations.push(operation);
    }
    operations
}

/// The labour menu grid filters by pricing and by whether an operation is on
/// the menu, sorts, pages and searches by code and name.
#[renox::test]
async fn aut_grid_the_labour_menu_filters_sorts_and_pages() {
    let t = boot().await;
    twelve_operations(&t).await;
    sign_in(&t, "Waleed", &[permissions::LABOUR_MENU_MANAGE]).await;

    let page = load(&t, "/auto/labour-menu?sort=-code&per_page=10").await;
    assert!(page.contains("1–10 of 12"));
    assert!(at(&page, "OP-11") < at(&page, "OP-10") && !has_row(&page, "OP-00"));
    assert!(page.contains("AED 180.00") && page.contains("Flat rate"));
    let second = load(&t, "/auto/labour-menu?sort=-code&per_page=10&page=2").await;
    assert!(second.contains("11–12 of 12") && has_row(&second, "OP-00"));

    let off = load(&t, "/auto/labour-menu?in.standing=off").await;
    assert!(off.contains("1–4 of 4") && has_row(&off, "OP-08") && !has_row(&off, "OP-07"));
    assert!(off.contains("Put back on the menu"));
    let on = load(&t, "/auto/labour-menu?in.standing=on").await;
    assert!(on.contains("Take off the menu"));
    let hourly = load(&t, "/auto/labour-menu?in.pricing=hourly").await;
    assert!(hourly.contains("1–4 of 4") && has_row(&hourly, "OP-11"));
    let found = load(&t, "/auto/labour-menu?search=operation%2003").await;
    assert!(has_row(&found, "OP-03") && !has_row(&found, "OP-04"));
}
