//! Base 07 *NCR lifecycle* (07 Manufacturing §7.1 *NCR*, `MFG.QCL.02`): an NCR
//! is raised on a lot by QC with a cause that defaults from the lot, it is
//! cancelled while `open`, QC changes its cause with a reason until it is
//! closed, and only the people with the NCR permissions of the plant see it.
//! The dispositions are BL-MFG-08 (`ncr_disposition.rs`).

mod support;

use manufacturing::{Cause, NcrStatus, Origin, Plant};
use renox::prelude::*;
use support::{Booted, boot};

const SUPPLIER: i64 = 7;

/// A plant, its QC inspector and lot 7 (100 kg from supplier 7, AED 30 a
/// kg, `rejected`) and lot 8 (made in-house, 50 kg).
async fn scene() -> (Booted, Plant, User) {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let qc = t.inspector("Ines", &plant).await;
    t.lots
        .put(7, "L7", Some(SUPPLIER), false, 100_000, 3_000, "rejected");
    t.lots.put(8, "F8", None, true, 50_000, 2_000, "quarantine");
    (t, plant, qc)
}

/// QC raises an NCR on lot L7 for 100 kg: it is `open`, numbered, takes the
/// lot's supplier and the cause *supplier* (incoming inspection), and the
/// date it was raised.
#[renox::test]
async fn an_ncr_on_a_supplier_lot_takes_the_supplier_and_the_cause_supplier() {
    let (t, plant, qc) = scene().await;

    let ncr = t.ncr(&qc, &plant, 7, "100").await;

    assert_eq!(ncr.status, NcrStatus::Open);
    assert!(ncr.number.starts_with("NCR-"), "{}", ncr.number);
    assert_eq!(ncr.supplier_party_id, Some(SUPPLIER));
    assert_eq!(ncr.cause, Cause::Supplier);
    assert_eq!(ncr.origin, Origin::Incoming);
    assert_eq!(ncr.quantity_milli, 100_000);
    assert_eq!(ncr.lot_number.as_deref(), Some("L7"));
    assert_eq!(ncr.raised_by, Some(qc.id));
    assert_eq!(ncr.defect, "colour off");
}

/// An NCR on a lot made in-house has no supplier and the cause *internal
/// process*; one on an item with no lot has the cause *unknown*.
#[renox::test]
async fn an_ncr_on_own_production_is_an_internal_cause() {
    let (t, plant, qc) = scene().await;

    let own = t.ncr(&qc, &plant, 8, "10").await;
    let plant_id = plant.id.to_string();
    t.post(
        &qc,
        "/manufacturing/ncrs",
        &[
            ("plant_id", &plant_id),
            ("item_id", "11"),
            ("quantity", "5"),
            ("defect", "short shot"),
        ],
    )
    .await
    .assert_status(303);
    let bare = t.ncr_now(own.id + 1).await;

    assert_eq!(own.supplier_party_id, None);
    assert_eq!(own.cause, Cause::InternalProcess);
    assert_eq!(bare.lot_id, None);
    assert_eq!(bare.cause, Cause::Unknown);
    assert_eq!(bare.item_id, 11);
}

/// A defect is described, the quantity is above zero and not more than the
/// lot holds, the lot is known, and a person without `ncrs.raise` in the
/// plant is refused.
#[renox::test]
async fn an_ncr_without_a_defect_or_with_too_much_is_refused() {
    let (t, plant, qc) = scene().await;
    let plant_id = plant.id.to_string();
    let raise = |lot: &'static str, kg: &'static str, defect: &'static str| {
        let (t, qc, plant_id) = (&t, &qc, &plant_id);
        async move {
            t.post(
                qc,
                "/manufacturing/ncrs",
                &[
                    ("plant_id", plant_id.as_str()),
                    ("lot_id", lot),
                    ("quantity", kg),
                    ("defect", defect),
                ],
            )
            .await
        }
    };

    let no_defect = raise("7", "10", " ").await;
    let too_much = raise("7", "100.5", "cracked").await;
    let none = raise("7", "0", "cracked").await;
    let unknown = raise("99", "1", "cracked").await;
    let outsider = t.user("Olga").await;
    let forbidden = t
        .post(
            &outsider,
            "/manufacturing/ncrs",
            &[
                ("plant_id", &plant_id),
                ("lot_id", "7"),
                ("quantity", "1"),
                ("defect", "x"),
            ],
        )
        .await;

    no_defect.assert_status(422);
    assert!(no_defect.text().contains("Describe the defect"));
    too_much.assert_status(422);
    assert!(
        too_much.text().contains("holds only 100"),
        "{}",
        too_much.text()
    );
    none.assert_status(422);
    unknown.assert_status(422);
    assert!(unknown.text().contains("not known"));
    forbidden.assert_status(403);
}

/// QC cancels an open NCR raised in error, with a reason; a cancelled NCR no
/// longer keeps its lot.
#[renox::test]
async fn an_open_ncr_is_cancelled_with_a_reason() {
    let (t, plant, qc) = scene().await;
    let ncr = t.ncr(&qc, &plant, 7, "10").await;

    let no_reason = t.ncr_act(&qc, &ncr, "cancel", &[("reason", " ")]).await;
    t.ncr_act(&qc, &ncr, "cancel", &[("reason", "wrong lot scanned")])
        .await
        .assert_status(303);
    let again = t.ncr_act(&qc, &ncr, "cancel", &[("reason", "twice")]).await;

    no_reason.assert_status(422);
    let cancelled = t.ncr_now(ncr.id).await;
    assert_eq!(cancelled.status, NcrStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("wrong lot scanned")
    );
    again.assert_status(422);
    assert!(!cancelled.keeps_lot());
}

/// QC changes the cause with a reason; a clerk who may raise changes it too, a
/// person who may not is refused, and a cancelled NCR keeps the one it had.
#[renox::test]
async fn qc_changes_the_cause_with_a_reason_until_the_ncr_is_closed() {
    let (t, plant, qc) = scene().await;
    let ncr = t.ncr(&qc, &plant, 7, "10").await;
    let outsider = t.user("Olga").await;

    let no_reason = t
        .ncr_act(
            &qc,
            &ncr,
            "cause",
            &[("cause", "internal_process"), ("reason", "")],
        )
        .await;
    let forbidden = t
        .ncr_act(
            &outsider,
            &ncr,
            "cause",
            &[("cause", "unknown"), ("reason", "x")],
        )
        .await;
    t.ncr_act(
        &qc,
        &ncr,
        "cause",
        &[
            ("cause", "internal_process"),
            ("reason", "dried too long on site"),
        ],
    )
    .await
    .assert_status(303);

    no_reason.assert_status(422);
    forbidden.assert_status(403);
    let changed = t.ncr_now(ncr.id).await;
    assert_eq!(changed.cause, Cause::InternalProcess);
    assert_eq!(
        changed.cause_reason.as_deref(),
        Some("dried too long on site")
    );
    assert_eq!(changed.cause_changed_by, Some(qc.id));
    assert!(changed.cause_changed_at.is_some());
    t.ncr_act(&qc, &changed, "cancel", &[("reason", "raised twice")])
        .await
        .assert_status(303);
    let late = t
        .ncr_act(
            &qc,
            &changed,
            "cause",
            &[("cause", "supplier"), ("reason", "again")],
        )
        .await;
    late.assert_status(422);
    assert!(late.text().contains("until the NCR is closed"));
}

/// The list and the record answer JSON to the people who work on NCRs in the
/// plant and 403 to anyone else.
#[renox::test]
async fn only_the_people_of_the_plant_see_the_ncrs() {
    let (t, plant, qc) = scene().await;
    let ncr = t.ncr(&qc, &plant, 7, "10").await;
    let other = t.plant("Plant 2").await;
    let elsewhere = t.inspector("Ivan", &other).await;
    let outsider = t.user("Olga").await;

    let list = t.get(&qc, "/manufacturing/ncrs").await;
    let none = t.get(&elsewhere, "/manufacturing/ncrs").await;
    let record = t.get(&qc, &format!("/manufacturing/ncrs/{}", ncr.id)).await;
    let hidden = t
        .get(&elsewhere, &format!("/manufacturing/ncrs/{}", ncr.id))
        .await;
    let blocked = t.get(&outsider, "/manufacturing/ncrs").await;

    list.assert_ok();
    assert!(list.text().contains(&ncr.number), "{}", list.text());
    none.assert_ok();
    assert!(!none.text().contains(&ncr.number));
    record.assert_ok();
    assert!(record.text().contains("colour off"));
    hidden.assert_status(403);
    blocked.assert_status(403);
}
