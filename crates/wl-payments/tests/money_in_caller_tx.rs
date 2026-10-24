//! BL-KRN-11 (#459): the cheques and deposits contracts join the caller's
//! transaction. `return_undeposited_in` and `start_settlement_in` move the
//! records inside a transaction the caller opened, commit nothing, and leave
//! the events on the caller's `AfterCommit`.

mod support;

use renox::prelude::*;
use renox::state_machine;
use wl_kernel::Subject;
use wl_kernel::after_commit::AfterCommit;
use wl_kernel::cheques::{ChequeId, ChequeStatus, ReturnCheques, cheques};
use wl_kernel::deposits::deposits;
use wl_payments::Cheque;
use wl_payments::deposit::DepositStatus;

use support::{BLUE_DUNES, boot};

/// The source the support holds deposits for.
fn lease() -> Subject {
    Subject::new("property.lease", 9)
}

/// Done when 3: `return_undeposited` in the caller's transaction returns 5
/// cheques against one receipt; on rollback the cheques are `received`
/// again and the receipt is recorded nowhere; committed, all 5 are
/// `returned` with that receipt.
#[renox::test]
async fn five_cheques_go_back_with_the_callers_commit_and_not_with_its_rollback() {
    let t = boot().await;
    let mut ids = Vec::new();
    for n in 0..5 {
        let cheque = t
            .received_cheque(BLUE_DUNES, &format!("40{n}"), 1_000_000, None)
            .await;
        ids.push(ChequeId(cheque.id));
    }
    let state = t.state();

    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    let done = cheques(state)
        .return_undeposited_in(
            state,
            &mut tx,
            &mut after,
            ReturnCheques::new(ids.clone(), "Lease terminated"),
            None,
        )
        .await
        .unwrap()
        .expect("five cheques went back");
    assert_eq!(done.returned, ids);
    assert_eq!(after.len(), 5, "one event per cheque, after the commit");
    let receipt = done.receipt.clone();
    // The caller's next step fails.
    drop(tx);
    drop(after);

    for id in &ids {
        assert_eq!(t.cheque(id.0).await.status, ChequeStatus::Received);
        let history = state_machine::history(t.db(), Cheque::TABLE, id.0)
            .await
            .unwrap();
        assert!(
            history
                .iter()
                .all(|row| row.metadata.get("receipt").is_none()),
            "the receipt {receipt} is recorded nowhere"
        );
    }

    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    let done = cheques(state)
        .return_undeposited_in(
            state,
            &mut tx,
            &mut after,
            ReturnCheques::new(ids.clone(), "Lease terminated"),
            None,
        )
        .await
        .unwrap()
        .expect("five cheques went back");
    tx.commit().await.unwrap();
    after.run(state).await.unwrap();
    for id in &ids {
        assert_eq!(t.cheque(id.0).await.status, ChequeStatus::Returned);
        let history = state_machine::history(t.db(), Cheque::TABLE, id.0)
            .await
            .unwrap();
        let last = history.last().expect("a history row");
        assert_eq!(last.metadata["receipt"], done.receipt.as_str());
    }
}

/// Done when 4: `start_settlement` in the caller's transaction moves the
/// deposit to `settlement_pending` only when the caller commits.
#[renox::test]
async fn the_settlement_starts_only_when_the_caller_commits() {
    let t = boot().await;
    let by = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 500_000).await;
    let deposit = t
        .hold_deposit(&by, BLUE_DUNES, 500_000, Some(&payment))
        .await;
    let state = t.state();

    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    let started = deposits(state)
        .start_settlement_in(state, &mut tx, &mut after, &lease())
        .await
        .unwrap()
        .expect("a deposit was held");
    assert_eq!(started.deposit.0, deposit.id);
    drop(tx);
    drop(after);
    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::Held);

    let mut tx = state.db.begin().await.unwrap();
    let mut after = AfterCommit::new();
    let started = deposits(state)
        .start_settlement_in(state, &mut tx, &mut after, &lease())
        .await
        .unwrap()
        .expect("a deposit was held");
    assert_eq!((started.held, started.forfeit, started.version), (500_000, 0, 0));
    tx.commit().await.unwrap();
    after.run(state).await.unwrap();
    assert_eq!(
        t.deposit(deposit.id).await.status,
        DepositStatus::SettlementPending
    );
}
