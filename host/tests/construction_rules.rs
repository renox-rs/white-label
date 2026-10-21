//! BL-CON-17 (#296) and BL-CON-18 (#297) behaviour under the 02 values: the
//! limits and thresholds of the Construction seed run by the real modules of
//! the host (a fresh Construction deployment), not read back as constants.

mod support;

use host::roles::{ACCOUNTANT, ADMIN, PROCUREMENT, PROJECT_MANAGER};
use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, entity, messages_to, noon_clock, person};
use wl_inventory::{Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use wl_invoicing::cash_alert::{self, BELOW_THRESHOLD};
use wl_invoicing::cash_balance::{self, EnterBalance};
use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::documents::today;
use wl_kernel::{ClientConfig, LegalEntity};

const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
trn = "100000000000003"
default = true
"#;

async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// The Monday of the week that holds today.
fn this_monday() -> NaiveDate {
    let today = today();
    today - Days::new(u64::from(today.weekday().num_days_from_monday()))
}

/// The first day of week `n` (from 1) of the forecast.
fn week(n: u64) -> NaiveDate {
    this_monday() + Days::new(7 * (n - 1))
}

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

async fn bill_due(app: &TestApp, entity: &LegalEntity, number: &str, due: NaiveDate, fils: i64) {
    let mut bill = SupplierBill {
        party_id: company(app, &format!("Vendor {number}")).await.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: support::day(-1),
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

async fn told(app: &TestApp, user: &User) -> usize {
    messages_to(app, BELOW_THRESHOLD, user).await.len()
}

/// Item 12: the 02 threshold is AED 200,000. A balance of AED 199,999
/// projected in week 5 alerts the MD and the accountant, once; AED 200,000
/// alerts no one.
#[renox::test]
async fn the_cash_threshold_is_two_hundred_thousand() {
    for (bill, alerts) in [(10_000_100_i64, 1_usize), (10_000_000, 0)] {
        let app = boot().await;
        let md = person(&app, "Omar", ADMIN).await;
        let accountant = person(&app, "Amal", ACCOUNTANT).await;
        let pm = person(&app, "Pam", PROJECT_MANAGER).await;
        SupplierBill::query().delete(app.db()).await.unwrap();
        let gbc = entity(&app, "GBC").await;
        // AED 300,000 in the bank; a bill falls due in week 5.
        bank(&app, &gbc, 30_000_000).await;
        bill_due(&app, &gbc, "SB-W5", week(5) + Days::new(2), bill).await;

        let sent = cash_alert::run(app.state(), today()).await.unwrap();

        assert_eq!(sent, alerts, "bill of {bill} fils");
        assert_eq!(told(&app, &md).await, alerts);
        assert_eq!(told(&app, &accountant).await, alerts);
        assert_eq!(told(&app, &pm).await, 0);
        if alerts == 1 {
            let notice = &messages_to(&app, BELOW_THRESHOLD, &md).await[0];
            assert_eq!(notice.data.0["week"], 5);
            assert_eq!(notice.data.0["balance"], "AED 199,999.00");
            assert_eq!(notice.data.0["threshold"], "AED 200,000.00");
            // The next daily run, with the same week below, says nothing.
            assert_eq!(cash_alert::run(app.state(), today()).await.unwrap(), 0);
            assert_eq!(told(&app, &md).await, 1);
        }
    }
}

/// Item 7: a PO of AED 8,000 revised to AED 15,000 is above the first limit
/// (AED 10,000) of 02: it waits for approval, the revision in force stays,
/// and a lower total applies at once.
#[renox::test]
async fn a_purchase_order_revised_from_8000_to_15000_needs_approval() {
    let app = boot().await;
    let buyer = person(&app, "Pia", PROCUREMENT).await;
    let supplier = company(&app, "Gulf Fasteners LLC").await;
    let mut item = Item::new("PARTS", "Spare parts", "pc");
    item.save(app.db()).await.unwrap();
    let mut store = Location::new("STORE-1", "Site store");
    store.save(app.db()).await.unwrap();
    let mut order = PurchaseOrder::new(supplier, store.id);
    order.status = PurchaseOrderStatus::Issued;
    let mut tx = app.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // 10 units at AED 800.
    let mut line = PurchaseOrderLine::new(order.id, 1, item.id, 10_000, 80_000);
    line.save(app.db()).await.unwrap();
    let qty = format!("qty_{}", line.id);
    let price = format!("price_{}", line.id);

    app.acting_as(&buyer);
    app.htmx()
        .post(
            &format!("/purchase-orders/{}/revise", order.id),
            &[
                ("reason", "price rise"),
                (qty.as_str(), "10"),
                (price.as_str(), "1500"),
            ],
        )
        .await
        .assert_ok();

    let now = PurchaseOrder::find_or_404(app.db(), order.id)
        .await
        .unwrap();
    assert_eq!(now.revision, 1, "the revision in force stays");
    let waiting = ApprovalRequest::pending_about(app.db(), &order.subject())
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].action, "inventory.purchase_order_revise");
    assert_eq!(waiting[0].requester_id, buyer.id);

    // AED 8,000 to AED 9,500 stays below the limit and applies at once (on
    // another order: a second revision waits while one is pending).
    let mut second = PurchaseOrder::new(supplier, store.id);
    second.status = PurchaseOrderStatus::Issued;
    let mut tx = app.db().begin().await.unwrap();
    second
        .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut other = PurchaseOrderLine::new(second.id, 1, item.id, 10_000, 80_000);
    other.save(app.db()).await.unwrap();
    app.htmx()
        .post(
            &format!("/purchase-orders/{}/revise", second.id),
            &[
                ("reason", "small rise"),
                (format!("qty_{}", other.id).as_str(), "10"),
                (format!("price_{}", other.id).as_str(), "950"),
            ],
        )
        .await
        .assert_ok();
    let applied = PurchaseOrder::find_or_404(app.db(), second.id)
        .await
        .unwrap();
    assert_eq!(applied.revision, 2, "AED 9,500 is below the limit");
}

// ---- Attendance under the 02 values (BL-CON-18, item 2).

use renox::chrono::{TimeZone, Utc};
use wl_attendance::AttendanceFlag;
use wl_attendance::service::{self, NewRecord};
use wl_attendance::{AttendanceRecord, WorkLocation};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_kernel::PartyId;
use wl_kernel::workforce::EmployeeId;

/// Moves the clock to 20:00 in Dubai on `date`, after the hours of the day.
fn travel_to_evening_of(app: &TestApp, date: NaiveDate) {
    let target = Utc.from_utc_datetime(&date.and_hms_opt(16, 0, 0).unwrap());
    let ahead = (target - renox::db::now()).num_seconds();
    assert!(ahead > 0, "{date} is in the future");
    app.travel(std::time::Duration::from_secs(ahead.unsigned_abs()));
}

/// `hour:minute` in Dubai on `date`.
fn dubai(date: NaiveDate, hour: u32, minute: u32) -> renox::db::DateTime {
    wl_kernel::BusinessCalendar::default().from_local(date.and_hms_opt(hour, minute, 0).unwrap())
}

/// An active labourer with a party of their own.
async fn labourer(app: &TestApp, name: &str) -> EmployeeId {
    let party: PartyId = support::tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "labourer", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    EmployeeId(employee.id)
}

async fn flags_of(app: &TestApp, record: &AttendanceRecord) -> Vec<String> {
    AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record.id))
        .order_by(AttendanceFlag::ID)
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|flag| flag.flag_type.as_str().to_owned())
        .collect()
}

/// A worker marked 11 hours on 2027-03-10 is flagged `overtime_cap`.
#[renox::test]
async fn eleven_hours_in_a_day_are_flagged_overtime_cap() {
    let app = boot().await;
    let supervisor = person(&app, "Sami", host::roles::SUPERVISOR).await;
    let worker = labourer(&app, "Wasim Khan").await;
    let site = WorkLocation::create(app.db(), WorkLocation::named("Site 1"))
        .await
        .unwrap();
    let date = NaiveDate::from_ymd_opt(2027, 3, 10).unwrap();
    travel_to_evening_of(&app, date);

    let record = app
        .at_travelled_time(async {
            service::record(
                app.state(),
                NewRecord::supervisor(
                    worker,
                    site.id,
                    dubai(date, 7, 0),
                    Some(dubai(date, 18, 0)),
                    supervisor.id,
                ),
            )
            .await
            .unwrap()
        })
        .await;

    let stored = AttendanceRecord::find_or_404(app.db(), record.id)
        .await
        .unwrap();
    assert!(
        flags_of(&app, &stored)
            .await
            .contains(&"overtime_cap".to_owned())
    );
}

/// On an outdoor site on 2027-07-10 hours from 11:00 to 16:00 are flagged
/// `midday_break` and the HSE officer is notified.
#[renox::test]
async fn hours_over_the_midday_break_outdoors_are_flagged_and_the_hse_officer_is_told() {
    let app = boot().await;
    let supervisor = person(&app, "Sami", host::roles::SUPERVISOR).await;
    let hse = person(&app, "Hamza", host::roles::HSE_OFFICER).await;
    let worker = labourer(&app, "Wasim Khan").await;
    let site = WorkLocation::create(app.db(), WorkLocation::named("Site 1").outdoors())
        .await
        .unwrap();
    let date = NaiveDate::from_ymd_opt(2027, 7, 10).unwrap();
    travel_to_evening_of(&app, date);

    let record = app
        .at_travelled_time(async {
            service::record(
                app.state(),
                NewRecord::supervisor(
                    worker,
                    site.id,
                    dubai(date, 11, 0),
                    Some(dubai(date, 16, 0)),
                    supervisor.id,
                ),
            )
            .await
            .unwrap()
        })
        .await;

    let stored = AttendanceRecord::find_or_404(app.db(), record.id)
        .await
        .unwrap();
    assert!(
        flags_of(&app, &stored)
            .await
            .contains(&"midday_break".to_owned())
    );
    let told = messages_to(&app, "attendance.record_flagged", &hse).await;
    assert!(
        told.iter().any(|m| m.data.0["flag"] == "midday_break"),
        "the HSE officer is told"
    );
}
