//! `Cheques::return_undeposited` (#251, wl-payments §4 `return`): the cheques
//! of an agreement that are not deposited go back against one handover
//! receipt; a deposited one is never touched.

mod support;

use renox::prelude::*;
use renox::state_machine;
use wl_kernel::cheques::{ChequeId, ChequeStatus, ReturnCheques, cheques};
use wl_payments::Cheque;

use support::{BLUE_DUNES, boot};

/// Two undeposited cheques are returned against one receipt, the deposited one
/// is skipped, and the history of the returned ones names the receipt.
#[renox::test]
async fn undeposited_cheques_go_back_against_one_receipt() {
    let t = boot().await;
    let staff = t.cheque_staff("Aisha").await;
    let one = t.received_cheque(BLUE_DUNES, "101", 2_000_000, None).await;
    let two = t.received_cheque(BLUE_DUNES, "102", 2_000_000, None).await;
    let three = t.received_cheque(BLUE_DUNES, "103", 2_000_000, None).await;
    let three = t.deposit_cheque(&staff, &three).await;
    assert_eq!(three.status, ChequeStatus::Deposited);

    let request = ReturnCheques::new(
        vec![ChequeId(one.id), ChequeId(two.id), ChequeId(three.id)],
        "Lease cancelled",
    );
    let done = cheques(t.state())
        .return_undeposited(t.state(), request, Some(staff.id))
        .await
        .unwrap()
        .expect("two cheques went back");
    assert_eq!(done.returned, [ChequeId(one.id), ChequeId(two.id)]);
    assert_eq!(done.skipped, [ChequeId(three.id)]);
    assert!(done.receipt.starts_with("HOR-"), "{}", done.receipt);

    assert_eq!(t.cheque(one.id).await.status, ChequeStatus::Returned);
    assert_eq!(t.cheque(two.id).await.status, ChequeStatus::Returned);
    assert_eq!(t.cheque(three.id).await.status, ChequeStatus::Deposited);

    for id in [one.id, two.id] {
        let history = state_machine::history(t.db(), Cheque::TABLE, id).await.unwrap();
        let last = history.last().expect("a history row");
        assert_eq!(last.transition, "return_by_system");
        assert_eq!(last.reason.as_deref(), Some("Lease cancelled"));
        assert_eq!(last.metadata["receipt"], done.receipt.as_str());
    }
}

/// Nothing to return gives `None`; a blank reason is refused.
#[renox::test]
async fn nothing_returnable_gives_none_and_a_reason_is_needed() {
    let t = boot().await;
    let staff = t.cheque_staff("Aisha").await;
    let cheque = t.received_cheque(BLUE_DUNES, "201", 1_000_000, None).await;
    let cheque = t.deposit_cheque(&staff, &cheque).await;
    let only_deposited = ReturnCheques::new(vec![ChequeId(cheque.id)], "Lease cancelled");
    assert!(
        cheques(t.state())
            .return_undeposited(t.state(), only_deposited, None)
            .await
            .unwrap()
            .is_none()
    );
    let blank = ReturnCheques::new(vec![ChequeId(cheque.id)], "  ");
    let refused = cheques(t.state())
        .return_undeposited(t.state(), blank, None)
        .await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}

/// `Cheques::relink` (#258): a received cheque pays another document, in the
/// same place; a deposited cheque, an unknown link and a repeat are refused.
#[renox::test]
async fn a_received_cheque_is_relinked_to_another_document() {
    use wl_kernel::Subject;

    let t = boot().await;
    let staff = t.cheque_staff("Aisha").await;
    let old = Subject::new("property.lease_charge", 5);
    let new = Subject::new("property.lease_charge", 9);
    let entity = t.entity("PGP").await;
    let request = wl_kernel::cheques::RegisterCheque::new(
        BLUE_DUNES,
        entity.id,
        "ENBD",
        "301",
        wl_kernel::documents::today(),
        1_000_000,
    )
    .link(old.clone());
    let cheque = t.register_cheque(request).await;

    cheques(t.state())
        .relink(t.state(), ChequeId(cheque.id), old.clone(), new.clone())
        .await
        .unwrap();
    let linked = cheque.linked(t.db()).await.unwrap();
    assert_eq!(linked, vec![new.clone()]);

    // The old link is gone, and a repeat is refused.
    for (from, to) in [(old.clone(), new.clone()), (new.clone(), new.clone())] {
        let refused = cheques(t.state())
            .relink(t.state(), ChequeId(cheque.id), from, to)
            .await;
        assert!(matches!(
            refused,
            Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
        ));
    }

    // A deposited cheque keeps its link.
    let deposited = t.deposit_cheque(&staff, &cheque).await;
    assert_eq!(deposited.status, ChequeStatus::Deposited);
    let refused = cheques(t.state())
        .relink(t.state(), ChequeId(cheque.id), new, old)
        .await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
}
