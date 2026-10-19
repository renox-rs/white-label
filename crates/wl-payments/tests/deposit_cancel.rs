//! `Deposits::held_for` and `Deposits::settle_cancelled` (#251, 01 Property
//! `PRP.ACL.18`): the settlement of the deposit of a cancelled lease starts
//! and the first version of the deductions is published, with the forfeited
//! part as a line that names its clause.

mod support;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::deposits::{CancelSettlement, DeductionKind, deposits};
use wl_payments::deposit::DepositStatus;

use support::{BLUE_DUNES, boot};

/// The source the support holds deposits for.
fn lease() -> Subject {
    Subject::new("property.lease", 9)
}

/// A held deposit is found by its source; none is found for another.
#[renox::test]
async fn a_held_deposit_is_found_by_its_source() {
    let t = boot().await;
    let by = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 500_000).await;
    let deposit = t.hold_deposit(&by, BLUE_DUNES, 500_000, Some(&payment)).await;

    let found = deposits(t.state())
        .held_for(t.db(), &lease())
        .await
        .unwrap()
        .expect("the deposit is held");
    assert_eq!(found.deposit.0, deposit.id);
    assert_eq!((found.required, found.held), (500_000, 500_000));
    assert_eq!(found.party, BLUE_DUNES);
    let other = Subject::new("property.lease", 10);
    assert!(deposits(t.state()).held_for(t.db(), &other).await.unwrap().is_none());
}

/// With nothing forfeited the settlement starts and a version with no line is
/// published, so the dispute window runs.
#[renox::test]
async fn the_settlement_starts_with_a_version_and_no_forfeit() {
    let t = boot().await;
    let by = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 500_000).await;
    let deposit = t.hold_deposit(&by, BLUE_DUNES, 500_000, Some(&payment)).await;

    let started = deposits(t.state())
        .settle_cancelled(t.state(), CancelSettlement::new(lease()), Some(by.id))
        .await
        .unwrap()
        .expect("a deposit was held");
    assert_eq!(started.deposit.0, deposit.id);
    assert_eq!((started.held, started.forfeit, started.version), (500_000, 0, 1));
    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::SettlementPending);

    let shown = t.published_for(BLUE_DUNES).await;
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].kind, DeductionKind::Deductions);
    assert!(shown[0].lines.is_empty());
    // Held no more: a second settlement finds nothing.
    let again = deposits(t.state())
        .settle_cancelled(t.state(), CancelSettlement::new(lease()), None)
        .await
        .unwrap();
    assert!(again.is_none());
}

/// A forfeit is one line that names the clause, and the party sees it.
#[renox::test]
async fn a_forfeit_is_published_as_a_deduction_with_its_clause() {
    let t = boot().await;
    let by = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 500_000).await;
    t.hold_deposit(&by, BLUE_DUNES, 500_000, Some(&payment)).await;

    let request = CancelSettlement::new(lease()).forfeit(200_000, "Reservation form §4");
    let started = deposits(t.state())
        .settle_cancelled(t.state(), request, Some(by.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.forfeit, 200_000);

    let shown = t.published_for(BLUE_DUNES).await;
    assert_eq!(shown[0].lines.len(), 1);
    assert_eq!(shown[0].lines[0].amount, 200_000);
    assert_eq!(shown[0].lines[0].evidence, "Reservation form §4");
    assert!(shown[0].lines[0].description.contains("Reservation form §4"));
}

/// Without a clause, or above the deposit, the forfeit is refused and the
/// deposit stays held.
#[renox::test]
async fn a_forfeit_without_a_clause_or_above_the_deposit_is_refused() {
    let t = boot().await;
    let by = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 500_000).await;
    let deposit = t.hold_deposit(&by, BLUE_DUNES, 500_000, Some(&payment)).await;

    let mut no_clause = CancelSettlement::new(lease());
    no_clause.forfeit = 100_000;
    let too_much = CancelSettlement::new(lease()).forfeit(600_000, "§4");
    for request in [no_clause, too_much] {
        let refused = deposits(t.state())
            .settle_cancelled(t.state(), request, None)
            .await;
        assert!(matches!(
            refused,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }
    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::Held);
}
