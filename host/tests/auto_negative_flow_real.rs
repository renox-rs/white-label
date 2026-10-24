//! BL-AUT-18 (#414): the 08 values of the shared negative-flow rules, run on
//! the REAL wl-inventory, wl-invoicing, wl-documents and kernel approvals of
//! the booted demo client, with the boundary of each value: the stock write-off
//! of AED 1,100 and the Return to supplier of AED 5,100 wait for a second
//! person while the limits themselves do not; the cash-flow alert of
//! `FIN.RUL.23` at AED 99,999 and none at AED 100,000; the supplier scorecard at
//! 69 and 70; the trademark notice 90 days before expiry; the late test drive
//! that waits for the workshop manager.

mod support;

use auto::late_times;
use auto::test_drives::NewDrive;
use host::roles::{
    ACCOUNTANT, BRANCH_MANAGER, OWNER, PROCUREMENT, SERVICE_ADVISOR, WORKSHOP_MANAGER,
};
use renox::chrono::{Datelike, Duration, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::autofx::{World, boot, inspected_card, sent_of};
use support::{company, messages_to, tenant};
use wl_inventory::movement::StockMovement;
use wl_inventory::return_credit::{CreditBasis, CreditOutcome, credit};
use wl_inventory::return_dispatch::{CollectionProof, DispatchOutcome, dispatch};
use wl_inventory::supplier_return::{NewReturn, NewReturnLine, SupplierReturn};
use wl_inventory::{InventoryConfig, Item, Location};
use wl_kernel::documents::today;
use wl_kernel::{LegalEntity, PartyId, Subject};

/// A Return to supplier of one unit of a new item, bought at `cost` fils.
async fn return_of(
    w: &World,
    code: &str,
    store: &Location,
    supplier: PartyId,
    cost: i64,
) -> SupplierReturn {
    let mut item = Item::new(code, code, "pcs");
    item.save(w.db()).await.unwrap();
    let mut received = StockMovement::receive(
        item.id,
        None,
        store.id,
        1_000,
        cost,
        cost,
        &Subject::new("inventory.goods_receipt", 900),
        None,
    );
    received.save(w.db()).await.unwrap();
    wl_inventory::supplier_return::create_from_stock(
        w.db(),
        None,
        NewReturn {
            supplier,
            source: Subject::new("inventory.stock_hold", 1),
            location_id: store.id,
            lines: vec![
                NewReturnLine::of(item.id, 1_000, "damaged")
                    .credit()
                    .at_cost(cost),
            ],
        },
    )
    .await
    .unwrap()
}

/// The stock limits run on the real returns: dispatching AED 5,000 needs no
/// one, AED 5,100 waits for a second person; a credit the supplier refused of
/// AED 1,000 posts at once, one of AED 1,100 waits (`STK.ACL.02`, `STK.ACL.06`).
#[renox::test]
async fn aut_18_real_the_stock_limits_hold_at_aed_1000_and_aed_5000() {
    let w = boot().await;
    let (_, pia) = w.staff("Pia", PROCUREMENT).await;
    let supplier = company(&w.app, "Gulf Auto Spares").await;
    let mut store =
        Location::new("WORKSHOP-1", "Workshop 1").in_scope(&Subject::new("scheduling.location", 1));
    store.save(w.db()).await.unwrap();
    let proof = CollectionProof::DriverSignature("SIG-1".to_owned());

    let at_limit = return_of(&w, "RTS-5000", &store, supplier, 500_000).await;
    assert!(matches!(
        dispatch(w.state(), &pia, at_limit.id, &proof)
            .await
            .unwrap(),
        DispatchOutcome::Dispatched(_)
    ));
    let above = return_of(&w, "RTS-5100", &store, supplier, 510_000).await;
    assert!(matches!(
        dispatch(w.state(), &pia, above.id, &proof).await.unwrap(),
        DispatchOutcome::Waiting(_)
    ));

    let small = return_of(&w, "WST-1000", &store, supplier, 100_000).await;
    dispatch(w.state(), &pia, small.id, &proof).await.unwrap();
    assert!(matches!(
        credit(
            w.state(),
            &pia,
            small.id,
            CreditBasis::SupplierRefused,
            "refused"
        )
        .await
        .unwrap(),
        CreditOutcome::Credited(_)
    ));
    let big = return_of(&w, "WST-1100", &store, supplier, 110_000).await;
    dispatch(w.state(), &pia, big.id, &proof).await.unwrap();
    assert!(matches!(
        credit(
            w.state(),
            &pia,
            big.id,
            CreditBasis::SupplierRefused,
            "refused"
        )
        .await
        .unwrap(),
        CreditOutcome::Waiting(_)
    ));
    let config = w.state().provided::<InventoryConfig>().expect("inventory");
    assert_eq!(config.wastage_approval_fils, 100_000);
    assert_eq!(config.return_approval_fils, 500_000);
}

/// A Monday-based week 5 of the forecast and the cash alert on the real
/// books: opening cash AED 150,000, a bill due in week 5; `FIN.RUL.23` run.
/// Returns how many notices went out and the world.
async fn cash_run(bill_fils: i64) -> (usize, World, Vec<i64>) {
    use wl_invoicing::cash_balance::{self, EnterBalance};
    use wl_invoicing::supplier_bill::{SupplierBill, SupplierBillStatus};

    let w = boot().await;
    let (omar, _) = w.staff("Omar", OWNER).await;
    let (aisha, _) = w.staff("Aisha", ACCOUNTANT).await;
    let entity = LegalEntity::default_entity(w.db()).await.unwrap();
    let on = today();
    cash_balance::enter(
        w.db(),
        None,
        &EnterBalance {
            legal_entity_id: entity.id,
            bank_account: "Main".to_owned(),
            amount: 15_000_000,
            as_of: on,
            note: None,
        },
    )
    .await
    .unwrap();
    let monday = on - Duration::days(i64::from(on.weekday().num_days_from_monday()));
    let due: NaiveDate = monday + Duration::days(4 * 7 + 2);
    // The demo seed has bills of its own: nothing but ours is owed.
    SupplierBill::query()
        .update(w.db(), &[("balance", &0_i64)])
        .await
        .unwrap();
    let supplier = company(&w.app, "Gulf Auto Spares").await;
    let mut bill = SupplierBill {
        party_id: supplier.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: "SB-W5".to_owned(),
        number_key: "sb-w5".to_owned(),
        bill_date: on,
        due_date: Some(due),
        currency: "AED".to_owned(),
        gross_total: bill_fils,
        balance: bill_fils,
        status: SupplierBillStatus::Approved,
        ..Default::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, SupplierBillStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let sent = wl_invoicing::cash_alert::run(w.state(), on).await.unwrap();
    (sent, w, vec![omar.id, aisha.id])
}

/// `FIN.RUL.23` with the 08 threshold of AED 100,000 per legal entity: AED 99,999
/// projected in week 5 alerts the Owner / GM and the accountant once (a second
/// run adds none); exactly AED 100,000 alerts nobody.
#[renox::test]
async fn aut_18_real_the_cash_flow_alert_fires_at_99999_and_not_at_100000() {
    // 150,000 - 50,001 = 99,999 in week 5.
    let (sent, w, both) = cash_run(5_000_100).await;
    assert_eq!(sent, 1);
    let told = sent_of(&w, wl_invoicing::cash_alert::BELOW_THRESHOLD).await;
    assert_eq!(
        told.len(),
        1 + 1,
        "the owner role holders and the accountant"
    );
    for user in &both {
        assert!(told.iter().any(|m| m.user_id == Some(*user)), "{user}");
    }
    assert_eq!(told[0].data.0["week"], 5);
    assert_eq!(told[0].data.0["threshold"], "AED 100,000.00");
    assert_eq!(told[0].data.0["balance"], "AED 99,999.00");
    assert_eq!(
        wl_invoicing::cash_alert::run(w.state(), today())
            .await
            .unwrap(),
        0,
        "once"
    );

    // 150,000 - 50,000 = 100,000: not below.
    let (sent, w, _) = cash_run(5_000_000).await;
    assert_eq!(sent, 0);
    assert!(
        sent_of(&w, wl_invoicing::cash_alert::BELOW_THRESHOLD)
            .await
            .is_empty()
    );
}

/// `STK.RUL.09` with the 08 threshold of 70: a snapshot of 69 opens a review
/// task for procurement, a snapshot of 70 does not.
#[renox::test]
async fn aut_18_real_a_supplier_scorecard_of_69_is_reviewed_and_70_is_not() {
    use wl_inventory::SupplierScore;
    use wl_inventory::supplier_review::{self, SupplierReview, task_key};
    use wl_inventory::supplier_score;

    let w = boot().await;
    let low = company(&w.app, "Gulf Auto Spares").await;
    let fine = company(&w.app, "Brake Parts FZE").await;
    let config = w.state().provided::<InventoryConfig>().expect("inventory");
    assert_eq!(config.score.threshold, 70);
    let date = today();
    let mut made = Vec::new();
    for (supplier, score) in [(low, 69), (fine, 70)] {
        let mut row = SupplierScore {
            supplier_party_id: supplier.0,
            snapshot_date: date,
            window_from: date - Duration::days(90),
            window_to: date - Duration::days(1),
            receipts: 5,
            on_time: 3,
            on_time_bp: Some(6_000),
            rejected_bp: Some(300),
            score: Some(score),
            band: supplier_score::band_of(Some(score), &config.score),
            computed_at: Some(renox::db::now()),
            ..SupplierScore::default()
        };
        row.save(w.db()).await.unwrap();
        made.push(row);
    }

    supplier_review::on_snapshots(w.state(), &made)
        .await
        .unwrap();

    let reviews = SupplierReview::query().get(w.db()).await.unwrap();
    assert_eq!(reviews.len(), 1, "only the 69");
    assert_eq!(reviews[0].supplier_party_id, low.0);
    let tasks = wl_ticketing::Ticket::query().get(w.db()).await.unwrap();
    assert!(
        tasks
            .iter()
            .any(|t| t.source_key.as_deref() == Some(task_key(low.0).as_str())),
        "the review task of the 69"
    );
    assert!(
        !tasks
            .iter()
            .any(|t| t.source_key.as_deref() == Some(task_key(fine.0).as_str()))
    );
}

/// A trademark registration expiring in 90 days notifies the Owner / GM on that
/// day; one expiring in 91 days does not yet.
#[renox::test]
async fn aut_18_real_a_trademark_expiring_in_90_days_notifies_the_owner() {
    use wl_documents::registrations::TRADEMARK;
    use wl_documents::{Document, DocumentKind, DocumentStatus, expiry};

    let w = boot().await;
    let (omar, _) = w.staff("Omar", OWNER).await;
    let entity = LegalEntity::default_entity(w.db()).await.unwrap();
    let kind = DocumentKind::by_key(w.db(), TRADEMARK)
        .await
        .unwrap()
        .expect("the trademark kind");
    for (title, days) in [("Mark A", 90), ("Mark B", 91)] {
        Document::create(
            w.db(),
            Document {
                subject_type: wl_documents::registrations::LEGAL_ENTITY_SUBJECT.into(),
                subject_id: entity.id,
                kind_id: kind.id,
                title: title.into(),
                expiry_date: Some(today() + Duration::days(days)),
                current_version: 1,
                status: DocumentStatus::Accepted,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    expiry::run(w.state(), today()).await.unwrap();

    // The 90-day step (offsets 180, 90 and 30) fires for the mark expiring in
    // 90 days only; the one expiring in 91 days has had its 180-day step.
    let told: Vec<_> = messages_to(&w.app, "documents.expiring", &omar)
        .await
        .into_iter()
        .filter(|m| m.data.0["offset"].as_i64() == Some(90))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].data.0["days_left"].as_i64(), Some(90));
    assert!(told[0].rendered_body.contains("90 day"));
}

/// A test drive entered 30 hours late waits in the inbox of the workshop manager
/// and the branch manager, not the advisor who entered it; nothing is written
/// until the workshop manager approves, then the drive is there with its
/// business time.
#[renox::test]
async fn aut_18_real_a_late_test_drive_waits_for_the_workshop_manager() {
    use wl_kernel::approvals;

    let w = boot().await;
    let (sami_user, sami) = w.staff("Sami", SERVICE_ADVISOR).await;
    let tareq = w.staff("Tareq", host::roles::TECHNICIAN).await;
    let (waleed_user, _) = w.staff("Waleed", WORKSHOP_MANAGER).await;
    let (basma_user, _) = w.staff("Basma", BRANCH_MANAGER).await;
    let owner = tenant(&w.app, "Mona Hassan").await;
    let card = inspected_card(&w, &sami, &tareq, owner, "12345", 0, vec![], false).await;
    let left = renox::db::now() - Duration::hours(30);

    let entered = late_times::enter(
        w.state(),
        &sami,
        card.id,
        NewDrive::out(52_210).left_at(left),
        Some("The system was down on Saturday"),
    )
    .await
    .unwrap();
    assert!(entered.is_pending());

    let waiting = approvals::awaiting(w.db(), &waleed_user).await.unwrap();
    assert_eq!(waiting.len(), 1, "the workshop manager has the card");
    assert_eq!(
        approvals::awaiting(w.db(), &basma_user)
            .await
            .unwrap()
            .len(),
        1,
        "the branch manager too"
    );
    assert!(
        approvals::awaiting(w.db(), &sami_user)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        auto::test_drive::TestDrive::of_job_card(w.db(), card.id)
            .await
            .unwrap()
            .is_empty()
    );
    approvals::approve(w.state(), waiting[0].id, &waleed_user, "agreed")
        .await
        .unwrap();
    let drives = auto::test_drive::TestDrive::of_job_card(w.db(), card.id)
        .await
        .unwrap();
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0].out_at.timestamp(), left.timestamp());
}
