//! Recording payments: the acceptance check AC2 of #94 (receipt numbers per
//! legal entity, one `PaymentReceived` each, and the refusals).

mod support;

use renox::prelude::Model;
use wl_kernel::payments::{
    Direction, PaymentMethod, PaymentStatus, RecordPayment, payment_recorder,
};
use wl_payments::{Payment, permissions as perm};

use support::{BLUE_DUNES, BLUE_DUNES_CODE, Booted, boot, year};

/// `POST /payments` as `user` with the form `pairs`.
async fn post_payment(
    t: &Booted,
    user: &renox::prelude::User,
    entity: i64,
    pairs: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let entity = entity.to_string();
    let mut fields = vec![("legal_entity_id", entity.as_str())];
    fields.extend_from_slice(pairs);
    t.app.acting_as(user);
    t.app.htmx().post("/payments", &fields).await
}

/// AC2: two incoming transfers for PGP are `RCT-PGP-<year>-0001` and
/// `-0002`, `received`, with one `PaymentReceived` each; a refused record in
/// between uses no number.
#[renox::test]
async fn transfers_get_gapless_receipt_numbers_per_entity() {
    let t = boot().await;
    let user = t.user_a().await;
    let pgp = t.entity("PGP").await;
    let transfer = [
        ("party_code", BLUE_DUNES_CODE),
        ("method", "transfer"),
        ("amount", "3000.00"),
        ("reference", "TRF-100"),
    ];

    post_payment(&t, &user, pgp.id, &transfer)
        .await
        .assert_status(303);
    // Refused: no reference. It must not use a number.
    post_payment(
        &t,
        &user,
        pgp.id,
        &[
            ("party_code", BLUE_DUNES_CODE),
            ("method", "transfer"),
            ("amount", "10.00"),
        ],
    )
    .await
    .assert_status(422);
    post_payment(&t, &user, pgp.id, &transfer)
        .await
        .assert_status(303);

    let payments = Payment::query()
        .order_by(Payment::ID)
        .get(t.db())
        .await
        .unwrap();
    let numbers: Vec<_> = payments.iter().map(|p| p.number.clone().unwrap()).collect();
    assert_eq!(
        numbers,
        [
            format!("RCT-PGP-{}-0001", year()),
            format!("RCT-PGP-{}-0002", year())
        ]
    );
    for payment in &payments {
        assert_eq!(payment.status, PaymentStatus::Received);
        assert_eq!(payment.direction, Direction::In);
        assert_eq!(payment.amount, 300_000);
        assert_eq!(payment.recorded_by, Some(user.id));
    }
    let events = t.received.all();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payment, payments[0].payment_id());
    assert_eq!(events[1].payment, payments[1].payment_id());
    assert_eq!(events[0].party, BLUE_DUNES);
    assert_eq!(events[0].amount, 300_000);
    assert_eq!(events[0].method, PaymentMethod::Transfer);
}

/// AC2: a transfer without a reference, an amount of 0 or less, a disabled
/// method and an unknown party are refused with 422 and leave no payment.
#[renox::test]
async fn bad_records_are_refused_and_leave_nothing() {
    let t = boot().await;
    let user = t.user_a().await;
    let pgp = t.entity("PGP").await;
    let refused: [&[(&str, &str)]; 5] = [
        &[
            ("party_code", BLUE_DUNES_CODE),
            ("method", "transfer"),
            ("amount", "5.00"),
        ],
        &[
            ("party_code", BLUE_DUNES_CODE),
            ("method", "cash"),
            ("amount", "0"),
        ],
        &[
            ("party_code", BLUE_DUNES_CODE),
            ("method", "cash"),
            ("amount", "-5.00"),
        ],
        &[
            ("party_code", BLUE_DUNES_CODE),
            ("method", "set_off"),
            ("amount", "5.00"),
        ],
        &[
            ("party_code", "P-99999"),
            ("method", "cash"),
            ("amount", "5.00"),
        ],
    ];
    for pairs in refused {
        post_payment(&t, &user, pgp.id, pairs)
            .await
            .assert_status(422);
    }
    assert_eq!(Payment::query().count(t.db()).await.unwrap(), 0);
    assert!(t.received.all().is_empty());

    // The same through the kernel contract: nothing is recorded, 422.
    let recorder = payment_recorder(t.state());
    let bad = [
        RecordPayment::new(BLUE_DUNES, pgp.id, PaymentMethod::Transfer, 500),
        RecordPayment::new(BLUE_DUNES, pgp.id, PaymentMethod::Cash, 0),
        RecordPayment::new(BLUE_DUNES, pgp.id, PaymentMethod::SetOff, 500),
        RecordPayment::new(wl_kernel::PartyId(99), pgp.id, PaymentMethod::Cash, 500),
    ];
    for request in bad {
        let refused = recorder.record(t.state(), request, None).await;
        assert!(
            matches!(
                refused,
                Err(renox::prelude::Error::Status(code, _)) if code == renox::prelude::StatusCode::UNPROCESSABLE_ENTITY
            ),
            "{refused:?}"
        );
    }
    assert_eq!(Payment::query().count(t.db()).await.unwrap(), 0);

    // A cash record needs no reference; the first number is still 0001.
    let cash = t
        .receive_request(RecordPayment::new(
            BLUE_DUNES,
            pgp.id,
            PaymentMethod::Cash,
            500,
        ))
        .await;
    assert_eq!(cash.number.unwrap(), format!("RCT-PGP-{}-0001", year()));
}

/// A user without `payments.record` cannot record.
#[renox::test]
async fn recording_needs_the_record_permission() {
    let t = boot().await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    let pgp = t.entity("PGP").await;
    post_payment(
        &t,
        &viewer,
        pgp.id,
        &[
            ("party_code", BLUE_DUNES_CODE),
            ("method", "cash"),
            ("amount", "5.00"),
        ],
    )
    .await
    .assert_forbidden();
    assert_eq!(Payment::query().count(t.db()).await.unwrap(), 0);
}
