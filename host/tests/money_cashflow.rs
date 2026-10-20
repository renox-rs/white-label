//! BL-PRP-26 (#276) item 25 with the real modules of the host: the cash-flow
//! alert (`FIN.RUL.23`) of a fresh 01 deployment. A projected balance of AED
//! 49,999 in week 5 alerts the company admin and the accountant once; AED
//! 50,000 in every week alerts no one; the forecast has 13 weekly buckets from
//! Monday; the check runs at 06:00 Dubai time; post-dated rent cheques count
//! at their cheque date and the published owner statements as payments (the
//! `CashForecastSource` of 01).

mod support;

use host::roles::{ACCOUNTANT, ADMIN, FINANCE_MANAGER, PROPERTY_MANAGER, RECEIVABLES};
use renox::chrono::{Datelike, Days, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::cash_forecast::{self, Forecast};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_kernel::LegalEntity;
use wl_kernel::cheques::{RegisterCheque, cheques};
use wl_kernel::documents::today;
use wl_kernel::invoicing::VatCategory;

/// The Monday of the week that holds today.
fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

/// The first day of week `n` (from 1) of the forecast.
fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

fn entities() -> Vec<String> {
    vec!["PGP".to_owned(), "PGFZ".to_owned()]
}

/// A bank balance of `fils` for `entity`, as of today.
async fn bank(app: &TestApp, entity: &LegalEntity, fils: i64) {
    cash_balance::enter(
        app.db(),
        None,
        &EnterBalance {
            legal_entity_id: entity.id,
            bank_account: "ENBD".to_owned(),
            amount: fils,
            as_of: today(),
            note: None,
        },
    )
    .await
    .unwrap();
}

/// An approved supplier bill of `fils` due `due`, payable by PGP.
async fn bill_due(app: &TestApp, entity: &LegalEntity, number: &str, due: NaiveDate, fils: i64) {
    let mut bill = SupplierBill {
        party_id: company(app, &format!("Vendor {number}")).await.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: day(-1),
        due_date: Some(due),
        currency: "AED".to_owned(),
        gross_total: fils,
        balance: fils,
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, SupplierBillStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// The alert state of a fresh deployment: both entities with their bank
/// balance, PGFZ comfortably above the threshold.
async fn scene() -> (TestApp, LegalEntity, User, User, User) {
    let app = booted().await;
    let admin = person(&app, "Alia", ADMIN).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    // Someone else: not told.
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    // The demo data has a bill of its own: the checks start from none.
    SupplierBill::query().delete(app.db()).await.unwrap();
    let pgp = entity(&app, "PGP").await;
    let pgfz = entity(&app, "PGFZ").await;
    bank(&app, &pgfz, 100_000_000).await;
    (app, pgp, admin, accountant, finance)
}

/// The users told by the alerts of PGP.
async fn told(app: &TestApp, user: &User) -> usize {
    messages_to(app, BELOW_THRESHOLD, user)
        .await
        .iter()
        .filter(|m| m.data.0["entity"] == "PGP")
        .count()
}

/// AED 49,999 projected in week 5 alerts the company admin and the
/// accountant, once; AED 50,000 alerts no one.
#[renox::test]
async fn forty_nine_thousand_nine_hundred_ninety_nine_in_week_five_alerts_once() {
    let (app, pgp, admin, accountant, finance) = scene().await;
    // AED 100,000 in the bank; a bill of AED 50,001 falls due in week 5.
    bank(&app, &pgp, 10_000_000).await;
    bill_due(&app, &pgp, "SB-W5", week(5) + Days::new(2), 5_000_100).await;

    let sent = cash_alert::run(app.state(), today()).await.unwrap();
    assert_eq!(sent, 1);
    assert_eq!(told(&app, &admin).await, 1);
    assert_eq!(told(&app, &accountant).await, 1);
    assert_eq!(told(&app, &finance).await, 0);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &admin).await[0];
    assert_eq!(notice.data.0["week"], 5);
    assert_eq!(notice.data.0["balance"], "AED 49,999.00");
    assert_eq!(notice.data.0["threshold"], "AED 50,000.00");

    // The next daily run, with the same first week below, says nothing.
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &admin).await, 1);
}

#[renox::test]
async fn fifty_thousand_in_every_week_alerts_no_one() {
    let (app, pgp, admin, accountant, _finance) = scene().await;
    bank(&app, &pgp, 10_000_000).await;
    // AED 50,000 of payments leaves exactly AED 50,000.
    bill_due(&app, &pgp, "SB-W5", week(5) + Days::new(2), 5_000_000).await;

    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
    assert_eq!(told(&app, &admin).await, 0);
    assert_eq!(told(&app, &accountant).await, 0);
}

/// The forecast has 13 weekly buckets from Monday and the check is scheduled
/// at 06:00 Asia/Dubai.
#[renox::test]
async fn the_forecast_has_thirteen_weeks_from_monday_and_runs_at_six() {
    let (app, pgp, _admin, _accountant, _finance) = scene().await;
    bank(&app, &pgp, 10_000_000).await;
    let module = host::invoicing_for("property", &entities());
    let config = module.config().clone();
    let inputs =
        cash_forecast_load::load(app.state(), &config, std::slice::from_ref(&pgp), today())
            .await
            .unwrap();
    let forecast: Forecast = cash_forecast::assemble(&inputs);
    let weeks = &forecast.weeks;
    assert_eq!(weeks.len(), 13);
    assert_eq!(weeks[0].start, this_monday());
    assert!(weeks.iter().all(|w| w.start.weekday() == Weekday::Mon));
    assert_eq!(weeks[12].start, week(13));

    // The scheduled task: 06:00 in Asia/Dubai.
    use renox::Module;
    let mut registry = renox::Registry::default();
    module.register(&mut registry);
    let run = registry
        .schedule()
        .upcoming(renox::timezone::Zone::UTC)
        .into_iter()
        .find(|run| run.name == cash_alert::TASK)
        .expect("the daily cash check is scheduled");
    let at = run.at.expect("it has a next run");
    assert_eq!(run.zone.to_string(), "Asia/Dubai");
    assert_eq!(
        run.zone.local(at.timestamp()).format("%H:%M").to_string(),
        "06:00"
    );
}

/// A post-dated rent cheque counts at its cheque date: dated in week 3 it
/// does not help week 2, dated in week 1 it does.
#[renox::test]
async fn a_post_dated_rent_cheque_counts_at_its_cheque_date() {
    for (cheque_week, alerts) in [(3, 1), (1, 0)] {
        let (app, pgp, admin, _accountant, _finance) = scene().await;
        let clerk = person(&app, "Clara", RECEIVABLES).await;
        // AED 60,000 in the bank, a bill of AED 50,001 in week 2: AED 9,999.
        bank(&app, &pgp, 6_000_000).await;
        bill_due(&app, &pgp, "SB-W2", week(2) + Days::new(1), 5_000_100).await;
        // A rent cheque of AED 45,000 for an invoice of the tenant.
        let tara = tenant(&app, "Tara Nasser").await;
        let invoice =
            issued_invoice(&app, &clerk, tara, 4_500_000, VatCategory::Exempt, day(30)).await;
        let request = RegisterCheque::new(
            tara,
            pgp.id,
            "ENBD",
            "9001",
            week(cheque_week) + Days::new(2),
            4_500_000,
        )
        .link(invoice.subject());
        cheques(app.state())
            .register(app.state(), request, None)
            .await
            .unwrap();

        let sent = cash_alert::run(app.state(), today()).await.unwrap();
        assert_eq!(sent, alerts, "cheque in week {cheque_week}");
        assert_eq!(told(&app, &admin).await, alerts);
        if alerts == 1 {
            let notice = &messages_to(&app, BELOW_THRESHOLD, &admin).await[0];
            assert_eq!(notice.data.0["week"], 2, "the cheque of week 3 is too late");
        }
    }
}

/// The published owner statements are payments of the forecast: a statement
/// of AED 10,000 takes a balance of AED 55,000 to AED 45,000, below the
/// threshold, while a disputed one is left out.
#[renox::test]
async fn a_published_owner_payout_counts_as_a_payment() {
    let (app, pgp, admin, accountant, _finance) = scene().await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    bank(&app, &pgp, 5_500_000).await;
    assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);

    let owner = tenant(&app, "Hessa Al Mansoori").await;
    let agreement = agreement_of(&app, owner).await;
    let published = published_statement(&app, &agreement, &accountant, &manager, 1_000_000).await;
    assert_eq!(published.balance(), 1_000_000);

    let sent = cash_alert::run(app.state(), today()).await.unwrap();
    assert_eq!(sent, 1, "AED 55,000 less the AED 10,000 payout");
    assert_eq!(told(&app, &admin).await, 1);
    let notice = &messages_to(&app, BELOW_THRESHOLD, &admin).await[0];
    assert_eq!(notice.data.0["week"], 1);
    assert_eq!(notice.data.0["balance"], "AED 45,000.00");
    assert!(
        notice.data.0["payments"]
            .as_str()
            .unwrap_or_default()
            .contains(&published.number()),
        "{}",
        notice.data.0["payments"]
    );
}
