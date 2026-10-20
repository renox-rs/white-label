//! A vendor contract bill against the real service contracts of wl-ticketing
//! (wl-invoicing §4 Supplier bill `submit`, §5.4 step 3; issue #87): the
//! coverage is the one the ticketing module provides, not a scripted one. A
//! terminated contract covers work up to its termination date, and a renewal
//! covers the days after the old end. The bill services run
//! `By::System(Some(user id))`, as in `supplier_bill_contract.rs`.

mod support;

use renox::auth::Permissions;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::duplicates::exceptions_of;
use wl_invoicing::supplier_bill::{self, BillDraft, WORK_AFTER_CONTRACT_END};
use wl_invoicing::{BillLineDraft, SupplierBill, SupplierBillStatus, WlInvoicing};
use wl_kernel::invoicing::VatCategory;
use wl_kernel::tickets::SERVICE_CONTRACT_SUBJECT_TYPE;
use wl_kernel::{ClientConfig, Subject};
use wl_ticketing::{ContractStatus, ServiceContract, WlTicketing};

use support::{BLUE_DUNES, Recording, TestDirectory};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Palm Grove Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing", "ticketing"]

[[legal_entities]]
code = "PGP"
name_en = "Palm Grove Properties LLC"
trn = "100000000000003"
default = true
"#;

const A: i64 = 7;

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

struct Real {
    app: TestApp,
    _views: tempfile::TempDir,
}

/// wl-invoicing and wl-ticketing in one app: the coverage is the real one.
async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let directory: std::sync::Arc<dyn wl_kernel::parties::PartyDirectory> =
        std::sync::Arc::new(TestDirectory::default());
    let notifier: std::sync::Arc<dyn wl_kernel::notify::Notifier> =
        std::sync::Arc::new(Recording::default());
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(WlInvoicing::new())
        .module(WlTicketing::new())
        .provide(directory)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    Real { app, _views: views }
}

/// A contract of Blue Dunes from 2026-01-01 to 2026-12-31 in `status`.
async fn contract(
    t: &Real,
    number: &str,
    status: ContractStatus,
    end: NaiveDate,
    previous: Option<i64>,
) -> ServiceContract {
    let mut contract = ServiceContract {
        number: number.to_owned(),
        vendor_party_id: BLUE_DUNES.0,
        title: "AMC".to_owned(),
        start_date: if previous.is_some() {
            date(10, 1)
        } else {
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        },
        end_date: end,
        previous_version_id: previous,
        status,
        ..ServiceContract::default()
    };
    let mut tx = t.app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    contract
}

/// A bill of Blue Dunes against `contract` with one line dated `day`,
/// submitted.
async fn submitted(
    t: &Real,
    number: &str,
    contract: &ServiceContract,
    day: NaiveDate,
) -> SupplierBill {
    let entity = wl_kernel::LegalEntity::query()
        .where_(wl_kernel::LegalEntity::CODE.eq("PGP"))
        .first(t.app.db())
        .await
        .unwrap()
        .unwrap();
    let mut draft = BillDraft::new(BLUE_DUNES, number, date(10, 6)).contract(
        Subject::new(SERVICE_CONTRACT_SUBJECT_TYPE, contract.id),
        None,
    );
    draft.legal_entity_id = Some(entity.id);
    let mut line = BillLineDraft::new("AMC visit", 80_000, VatCategory::Standard);
    line.service_date = Some(day);
    let bill = supplier_bill::create(t.app.db(), &draft.line(line), Some(A))
        .await
        .unwrap();
    submit(t.app.state(), By::System(Some(A)), bill.id)
        .await
        .unwrap()
}

/// SC-12 terminated with termination date 2026-09-15 (its end date was
/// 2026-12-31): a line dated 2026-09-20 sends the bill to `exception`, one
/// dated 2026-09-15 is `matched`. The contract is the one wl-ticketing keeps.
#[renox::test]
async fn a_real_terminated_contract_covers_work_up_to_its_termination_date() {
    let t = boot().await;
    let mut sc12 = contract(
        &t,
        "AMC-2026-0012",
        ContractStatus::Terminated,
        date(12, 31),
        None,
    )
    .await;
    sc12.termination_date = Some(date(9, 15));
    sc12.save(t.app.db()).await.unwrap();

    let late = submitted(&t, "R-1", &sc12, date(9, 20)).await;
    assert_eq!(late.status, SupplierBillStatus::Exception);
    let rows = exceptions_of(t.app.db(), late.id).await.unwrap();
    assert_eq!(rows[0].reason_code, WORK_AFTER_CONTRACT_END);
    assert!(rows[0].detail.clone().unwrap().contains("2026-09-20"));

    let on_time = submitted(&t, "R-2", &sc12, date(9, 15)).await;
    assert_eq!(on_time.status, SupplierBillStatus::Matched);
}

/// A contract ending on 2026-09-30 renewed from 2026-10-01: a line of
/// 2026-10-05 is covered by the renewal when the bill names the old contract.
#[renox::test]
async fn a_renewal_covers_the_work_after_the_old_end() {
    let t = boot().await;
    let old = contract(
        &t,
        "AMC-2026-0001",
        ContractStatus::Active,
        date(9, 30),
        None,
    )
    .await;
    let held = submitted(&t, "R-3", &old, date(10, 5)).await;
    assert_eq!(held.status, SupplierBillStatus::Exception);

    contract(
        &t,
        "AMC-2026-0002",
        ContractStatus::Active,
        NaiveDate::from_ymd_opt(2027, 9, 30).unwrap(),
        Some(old.id),
    )
    .await;
    let covered = submitted(&t, "R-4", &old, date(10, 5)).await;
    assert_eq!(covered.status, SupplierBillStatus::Matched);
}
