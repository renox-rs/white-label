//! BL-DLV-02 (#158), `WRK.POD.01`, `X-11`: the recipient may accept some
//! lines, or part of a line, and refuse the rest. The driver records per line
//! the delivered and refused quantity with a refusal reason; the delivery is
//! `delivered` with those quantities and `DeliveryDelivered` carries them.
//! Refusing everything is a failed attempt.

mod support;

use wl_delivery::{Attempt, Delivery, DeliveryLine, DeliveryStatus, Proof, WlDelivery};
use wl_kernel::DeliveredLine;

use support::documents::Documents;
use support::{World, boot, boot_with, stop_fields, without};

/// A delivery of line A (10, AED 100) and line B (5, AED 50) that is out for
/// delivery, with `collect` fils to collect and the amounts per line when
/// `amounts` says so.
async fn two_lines(
    w: &World,
    collect: i64,
    amounts: bool,
) -> (Delivery, DeliveryLine, DeliveryLine) {
    let (a, b) = if amounts {
        (Some(10_000), Some(5_000))
    } else {
        (None, None)
    };
    let delivery = w
        .request(&[("Tiles", 10_000, a), ("Grout", 5_000, b)], collect)
        .await;
    let out = w.out_for_delivery(&delivery).await;
    let mut lines = w.lines(&out).await;
    let second = lines.remove(1);
    let first = lines.remove(0);
    (out, first, second)
}

async fn attempts_of(w: &World, delivery: &Delivery) -> u64 {
    Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery.id))
        .count(w.db())
        .await
        .unwrap()
}

/// AC1: line A qty 10 and line B qty 5, photo and signature captured; A
/// delivered 10, B delivered 3 and refused 2 *damaged* with a photo: the
/// delivery is `delivered` and `DeliveryDelivered` carries A 10/0 and B 3/2
/// *damaged*.
#[renox::test]
async fn a_partly_accepted_delivery_is_delivered_with_the_quantities_per_line() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let fields = stop_fields(
        &[
            (a.id, "10", "0", "", ""),
            (b.id, "3", "2", "damaged", "damage-photo-1"),
        ],
        "0",
    );

    w.deliver(&w.driver, &delivery, &fields).await;

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    let lines = w.lines(&after).await;
    assert_eq!((lines[0].delivered_milli, lines[0].refused_milli), (10_000, 0));
    assert_eq!((lines[1].delivered_milli, lines[1].refused_milli), (3_000, 2_000));
    assert_eq!(lines[1].refusal_reason.as_deref(), Some("damaged"));
    assert_eq!(lines[1].refusal_photo_ref.as_deref(), Some("damage-photo-1"));

    let events = w.delivered.all();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.delivery, delivery.delivery_id());
    assert_eq!(event.source, delivery.source());
    assert_eq!(
        event.lines,
        vec![
            DeliveredLine {
                line: None,
                description: "Tiles".into(),
                quantity_milli: 10_000,
                delivered_milli: 10_000,
                refused_milli: 0,
                refusal_reason: None,
            },
            DeliveredLine {
                line: None,
                description: "Grout".into(),
                quantity_milli: 5_000,
                delivered_milli: 3_000,
                refused_milli: 2_000,
                refusal_reason: Some("damaged".into()),
            },
        ]
    );
    // The proof carries the methods, the recipient, the time and the place.
    let proof = after.latest_proof(w.db()).await.unwrap().unwrap();
    assert_eq!(proof.methods, "photo,signature");
    assert_eq!(proof.recipient_name, "Mariam Al Hosani");
    let attempts = after.attempts_made(w.db()).await.unwrap();
    assert!(attempts[0].latitude_micro.is_some() && attempts[0].longitude_micro.is_some());
    assert_eq!(event.proof, proof.id);
}

/// AC2: the default setting needs photo and signature: only a photo
/// captured, `deliver` is refused and nothing is saved.
#[renox::test]
async fn deliver_is_refused_without_the_signature_the_setting_requires() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let fields = without(
        stop_fields(&[(a.id, "10", "0", "", ""), (b.id, "5", "0", "", "")], "0"),
        "signature",
    );

    w.deliver(&w.driver, &delivery, &fields)
        .await
        .assert_status(422)
        .assert_see("signature");

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);
    assert_eq!(attempts_of(&w, &delivery).await, 0);
    assert!(w.delivered.all().is_empty());
    let lines = w.lines(&delivery).await;
    assert_eq!(lines[0].delivered_milli, 0);
}

/// AC3: line B refused 2 without a reason: `deliver` is refused.
#[renox::test]
async fn deliver_is_refused_when_a_refused_quantity_has_no_reason() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let fields = stop_fields(&[(a.id, "10", "0", "", ""), (b.id, "3", "2", "", "")], "0");

    w.deliver(&w.driver, &delivery, &fields)
        .await
        .assert_status(422)
        .assert_see("refusal reason");

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);
    assert!(w.delivered.all().is_empty());
}

/// AC4: delivered plus refused is 4 for a line of 5: `deliver` is refused.
#[renox::test]
async fn deliver_is_refused_when_delivered_plus_refused_is_not_the_quantity() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let fields = stop_fields(
        &[(a.id, "10", "0", "", ""), (b.id, "2", "2", "other", "")],
        "0",
    );

    w.deliver(&w.driver, &delivery, &fields)
        .await
        .assert_status(422)
        .assert_see("must equal the quantity ordered (5)");

    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);
}

/// AC5: a refused quantity with the reason *damaged* and no photo: `deliver`
/// is refused; another reason needs no photo.
#[renox::test]
async fn deliver_is_refused_when_a_damaged_quantity_has_no_photo() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let damaged = stop_fields(
        &[(a.id, "10", "0", "", ""), (b.id, "3", "2", "damaged", "")],
        "0",
    );
    w.deliver(&w.driver, &delivery, &damaged)
        .await
        .assert_status(422)
        .assert_see("damaged quantity needs a photo");
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);

    let wrong_item = stop_fields(
        &[(a.id, "10", "0", "", ""), (b.id, "3", "2", "wrong_item", "")],
        "0",
    );
    w.deliver(&w.driver, &delivery, &wrong_item).await;
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::Delivered);
}

/// AC6: every line refused in full: `deliver` is refused, and `fail` with the
/// reason *refused* moves the delivery to `attempted`.
#[renox::test]
async fn refusing_every_line_is_a_failed_attempt_with_the_reason_refused() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let fields = stop_fields(
        &[(a.id, "0", "10", "not_ordered", ""), (b.id, "0", "5", "not_ordered", "")],
        "0",
    );

    w.deliver(&w.driver, &delivery, &fields)
        .await
        .assert_status(422)
        .assert_see("Nothing was accepted");
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);

    w.fail(&w.driver, &delivery, "refused").await;
    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Attempted);
    let failed = w.failed.all();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].reason, "refused");
    assert!(w.delivered.all().is_empty());
}

/// AC7: a source type with partial acceptance off: refused quantities cannot
/// be entered (the stop screen hides the fields), and a line in full is
/// delivered.
#[renox::test]
async fn a_source_type_with_partial_acceptance_off_takes_no_refused_quantity() {
    let w = boot_with(
        WlDelivery::new().partial_acceptance("logistics.shipment", false),
        Documents::logistics(),
    )
    .await;
    let parcel = w
        .request_from("logistics.shipment", &[("Parcel", 1_000, None)], 0)
        .await;
    assert!(!parcel.partial_acceptance);
    let out = w.out_for_delivery(&parcel).await;
    let line = &w.lines(&out).await[0];

    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", out.id))
        .await
        .assert_ok()
        .assert_dont_see("name=\"refused_");

    let refused = stop_fields(&[(line.id, "0.5", "0.5", "other", "")], "0");
    w.deliver(&w.driver, &out, &refused)
        .await
        .assert_status(422)
        .assert_see("Partial acceptance is off");
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::OutForDelivery);

    w.deliver(&w.driver, &out, &stop_fields(&[(line.id, "1", "0", "", "")], "0"))
        .await;
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
}

/// AC7, the other side: a source type with partial acceptance on shows the
/// refused fields.
#[renox::test]
async fn the_stop_screen_shows_the_refused_fields_where_partial_acceptance_is_on() {
    let w = boot().await;
    let (delivery, _a, _b) = two_lines(&w, 0, false).await;
    w.app.acting_as(&w.driver);
    w.app
        .get(&format!("/driver/stops/{}", delivery.id))
        .await
        .assert_ok()
        .assert_see("name=\"refused_");
}

/// AC8: a `delivered` delivery: the proof and the line quantities cannot be
/// edited by any role. A second submit is refused and shows the saved proof;
/// no other action changes the delivered record either.
#[renox::test]
async fn a_delivered_proof_and_its_quantities_are_never_edited() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 0, false).await;
    let first = stop_fields(
        &[(a.id, "10", "0", "", ""), (b.id, "3", "2", "other", "")],
        "0",
    );
    w.deliver(&w.driver, &delivery, &first).await;
    let before_lines = w.lines(&delivery).await;
    let before_proof: Proof = w
        .reload(&delivery)
        .await
        .latest_proof(w.db())
        .await
        .unwrap()
        .unwrap();

    // The driver submits again, with other quantities.
    let second = stop_fields(
        &[(a.id, "4", "6", "other", ""), (b.id, "5", "0", "", "")],
        "0",
    );
    w.deliver(&w.driver, &delivery, &second)
        .await
        .assert_status(422)
        .assert_see("already delivered")
        .assert_see("Mariam Al Hosani");
    // A failed visit cannot be recorded over it either.
    w.fail(&w.driver, &delivery, "not_home").await.assert_status(422);
    // The dispatcher and the manager cannot take it off a run or put it on
    // another.
    for user in [&w.dispatcher, &w.manager] {
        w.app.acting_as(user);
        w.app
            .htmx()
            .post(&format!("/deliveries/{}/unassign", delivery.id), &[])
            .await
            .assert_status(422);
    }

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    assert_eq!(after.attempts, 1);
    let after_lines = w.lines(&after).await;
    for (was, now) in before_lines.iter().zip(&after_lines) {
        assert_eq!(was.delivered_milli, now.delivered_milli);
        assert_eq!(was.refused_milli, now.refused_milli);
        assert_eq!(was.refusal_reason, now.refusal_reason);
    }
    let after_proof = after.latest_proof(w.db()).await.unwrap().unwrap();
    assert_eq!(after_proof.id, before_proof.id);
    assert_eq!(after_proof.recipient_name, before_proof.recipient_name);
    assert_eq!(after_proof.methods, before_proof.methods);
    assert_eq!(attempts_of(&w, &after).await, 1);
    assert_eq!(w.delivered.all().len(), 1);
}

/// AC9: cash on delivery with line amounts, A AED 100 (qty 10) and B AED 50
/// (qty 5), B delivered 3 and refused 2: the expected cash is AED 130, and
/// collecting AED 130 needs no difference reason.
#[renox::test]
async fn the_expected_cash_is_reduced_by_the_refused_lines_amounts() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 15_000, true).await;
    let fields = stop_fields(
        &[(a.id, "10", "0", "", ""), (b.id, "3", "2", "other", "")],
        "130.00",
    );

    w.deliver(&w.driver, &delivery, &fields).await;

    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    let proof = after.latest_proof(w.db()).await.unwrap().unwrap();
    assert_eq!(proof.cash_expected, 13_000);
    assert_eq!(proof.cash_collected, 13_000);
    let event = &w.delivered.all()[0];
    assert_eq!(event.cash_expected, 13_000);
    assert_eq!(event.cash_collected, 13_000);
    assert_eq!(event.cod_difference, 0);
    assert_eq!(event.cod_difference_reason, None);
}

/// AC9, the edge: a part of a line is taken pro rata to its quantity, and
/// more cash than expected is refused (the change is given back).
#[renox::test]
async fn cash_above_the_expected_amount_is_refused() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 15_000, true).await;
    let fields = stop_fields(
        &[(a.id, "10", "0", "", ""), (b.id, "3", "2", "other", "")],
        "150.00",
    );
    w.deliver(&w.driver, &delivery, &fields)
        .await
        .assert_status(422)
        .assert_see("give the change back");
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);
}

/// AC10: cash on delivery of AED 150 without line amounts and the same
/// refusal: the expected cash stays AED 150, so collecting AED 130 is a
/// shortfall (the difference flow, BL-DLV-03) and AED 150 is accepted.
#[renox::test]
async fn without_line_amounts_the_expected_cash_stays_the_full_amount() {
    let w = boot().await;
    let (delivery, a, b) = two_lines(&w, 15_000, false).await;
    let lines = [(a.id, "10", "0", "", ""), (b.id, "3", "2", "other", "")];

    w.deliver(&w.driver, &delivery, &stop_fields(&lines, "130.00"))
        .await
        .assert_status(422)
        .assert_see("short of the AED 150.00 expected");
    assert_eq!(w.reload(&delivery).await.status, DeliveryStatus::OutForDelivery);

    w.deliver(&w.driver, &delivery, &stop_fields(&lines, "150.00"))
        .await;
    let after = w.reload(&delivery).await;
    assert_eq!(after.status, DeliveryStatus::Delivered);
    let proof = after.latest_proof(w.db()).await.unwrap().unwrap();
    assert_eq!(proof.cash_expected, 15_000);
    assert_eq!(w.delivered.all()[0].cash_expected, 15_000);
}

/// The required proof is the setting of the source type, copied onto the
/// delivery: a source type that accepts any one method needs only one.
#[renox::test]
async fn a_source_type_that_accepts_any_method_needs_only_one() {
    use wl_kernel::PodMethod;
    let w = boot_with(
        WlDelivery::new().required_proof(
            "inventory.dispatch_note",
            &[PodMethod::Signature, PodMethod::Otp],
            true,
        ),
        Documents::logistics(),
    )
    .await;
    let delivery = w
        .request_from("inventory.dispatch_note", &[("Pump", 2_000, None)], 0)
        .await;
    assert_eq!(delivery.pod_methods, "signature,otp");
    assert!(delivery.pod_any);
    let out = w.out_for_delivery(&delivery).await;
    let line = &w.lines(&out).await[0];

    // Neither method: refused. The photo of the default setting is not one.
    let none = without(
        without(stop_fields(&[(line.id, "2", "0", "", "")], "0"), "signature"),
        "nothing",
    );
    w.deliver(&w.driver, &out, &none)
        .await
        .assert_status(422)
        .assert_see("The proof is incomplete");
    w.deliver(&w.driver, &out, &stop_fields(&[(line.id, "2", "0", "", "")], "0"))
        .await;
    assert_eq!(w.reload(&out).await.status, DeliveryStatus::Delivered);
}
