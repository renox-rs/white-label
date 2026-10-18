//! The module as an app boots it: the acceptance checks AC1 and AC3 of #94
//! (the list needs `payments.view`; a payment is born `received` or
//! `pending` and its status moves only by a transition).

mod support;

use renox::prelude::*;
use wl_kernel::payments::{PaymentStatus, payment_recorder};
use wl_payments::{Payment, permissions as perm};

use support::{BLUE_DUNES, boot};

/// AC1: `/payments` answers 200 with `payments.view` and 403 without it, and
/// the module provides its recorder (the kernel finds the payment).
#[renox::test]
async fn the_payment_list_needs_the_view_permission() {
    let t = boot().await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    let nobody = t.user("Nora", &[]).await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    assert_eq!(
        payment.number.as_deref(),
        Some(&*format!("RCT-PGP-{}-0001", support::year()))
    );

    t.app.acting_as(&viewer);
    t.app
        .get("/payments")
        .await
        .assert_ok()
        .assert_see(&format!("RCT-PGP-{}-0001", support::year()));
    t.app.acting_as(&nobody);
    t.app.get("/payments").await.assert_forbidden();

    let found = payment_recorder(t.state())
        .find(t.db(), payment.payment_id())
        .await
        .unwrap()
        .expect("the recorder finds the payment");
    assert_eq!(found.status, PaymentStatus::Received);
    assert_eq!(found.amount, 300_000);
}

#[renox::test]
async fn the_models_match_the_schema_and_every_view_compiles() {
    let t = boot().await;
    t.app.assert_models_match_schema().await;
    t.app.assert_views_compile();
}

/// AC3: a payment cannot be inserted in a state other than `received` or
/// `pending`, and a changed status is refused outside a transition.
#[renox::test]
async fn the_status_moves_only_by_transition() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;

    let mut changed = payment.clone();
    changed.status = PaymentStatus::Reversed;
    assert!(changed.save(t.db()).await.is_err());
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Received);

    for status in [
        PaymentStatus::Reversed,
        PaymentStatus::Failed,
        PaymentStatus::ChargedBack,
    ] {
        let mut born = Payment {
            number: Some(format!("RCT-PGP-9999-{status:?}")),
            legal_entity_id: payment.legal_entity_id,
            party_id: BLUE_DUNES.0,
            amount: 100,
            currency: "AED".to_owned(),
            status,
            ..Payment::default()
        };
        assert!(born.insert(t.db()).await.is_err(), "{status:?}");
    }
    for status in [PaymentStatus::Received, PaymentStatus::Pending] {
        let mut born = Payment {
            number: Some(format!("RCT-PGP-9999-{status:?}")),
            legal_entity_id: payment.legal_entity_id,
            party_id: BLUE_DUNES.0,
            amount: 100,
            currency: "AED".to_owned(),
            status,
            ..Payment::default()
        };
        born.insert(t.db())
            .await
            .unwrap_or_else(|e| panic!("{status:?}: {e}"));
    }
}
