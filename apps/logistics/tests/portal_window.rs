//! BL-LOG-26 (#324), 03 §3.3 `POR.ACL.01`: after a client's contract ends its
//! portal users keep read-only access to shipments, invoices and COD
//! statements for 90 days, then the portal closes. The end of the contract is
//! recorded on the client's terms; a client user of the company is the
//! portal's `noor`.

mod support;

use logistics::portal::{self, Access};
use renox::chrono::TimeDelta;
use support::{SOUQ, boot};
use wl_kernel::documents::today;

/// `days` before today as `2026-11-01`.
fn ago(days: i64) -> String {
    (today() - TimeDelta::days(days)).to_string()
}

/// The ops user records the end of the client's contract on `date`.
async fn contract_ended(w: &support::World, date: &str) {
    w.post(
        &w.ops,
        &format!("/clients/{}/contract-end", SOUQ.0),
        &[("date", date)],
    )
    .await
    .assert_status(303);
}

/// An open contract: the client user books, cancels and looks.
#[renox::test]
async fn while_the_contract_runs_the_client_user_acts_and_looks() {
    let w = boot().await;
    let job = w.open_job().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    w.get(&w.noor, &format!("/freight/{}", job.id))
        .await
        .assert_ok();
    w.post(&w.noor, &format!("/shipments/{}/cancel", shipment.id), &[])
        .await
        .assert_status(303);
    assert_eq!(
        portal::access(w.db(), SOUQ, today()).await.unwrap(),
        Access::Open
    );
}

/// Read-only for 90 days after the end: the client user looks at what the
/// company has and can no longer book or cancel.
#[renox::test]
async fn after_the_end_the_client_user_only_looks_for_90_days() {
    let w = boot().await;
    let job = w.open_job().await;
    let shipment = w.book_as(&w.noor, SOUQ, "ORD-1").await;
    contract_ended(&w, &ago(10)).await;
    assert_eq!(
        portal::access(w.db(), SOUQ, today()).await.unwrap(),
        Access::ReadOnly
    );
    w.get(&w.noor, &format!("/freight/{}", job.id))
        .await
        .assert_ok();
    w.try_book(&w.noor, SOUQ, "ORD-2", &[])
        .await
        .assert_forbidden();
    w.post(&w.noor, &format!("/shipments/{}/cancel", shipment.id), &[])
        .await
        .assert_forbidden();
    // Staff are not affected.
    w.get(&w.docs, &format!("/freight/{}", job.id))
        .await
        .assert_ok();
    // The last day of the window still looks; the contract renewed opens it.
    contract_ended(&w, &ago(90)).await;
    w.get(&w.noor, &format!("/freight/{}", job.id))
        .await
        .assert_ok();
    contract_ended(&w, "").await;
    w.book_as(&w.noor, SOUQ, "ORD-3").await;
}

/// Closed from the day after the 90 days: the client user sees nothing.
#[renox::test]
async fn after_90_days_the_portal_is_closed() {
    let w = boot().await;
    let job = w.open_job().await;
    contract_ended(&w, &ago(91)).await;
    assert_eq!(
        portal::access(w.db(), SOUQ, today()).await.unwrap(),
        Access::Closed
    );
    w.get(&w.noor, &format!("/freight/{}", job.id))
        .await
        .assert_forbidden();
    w.try_book(&w.noor, SOUQ, "ORD-9", &[])
        .await
        .assert_forbidden();
}

/// Only a holder of `shipments.manage` records the end; the date is checked.
#[renox::test]
async fn only_staff_who_manage_shipments_record_the_end_of_a_contract() {
    let w = boot().await;
    let path = format!("/clients/{}/contract-end", SOUQ.0);
    w.post(&w.docs, &path, &[("date", "2026-11-01")])
        .await
        .assert_forbidden();
    w.post(&w.noor, &path, &[("date", "2026-11-01")])
        .await
        .assert_forbidden();
    w.post(&w.ops, &path, &[("date", "next week")])
        .await
        .assert_status(422);
}
