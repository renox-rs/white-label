//! BL-MFG-15 (#396): the NCRs caused by the supplier reach the supplier
//! scorecard through the kernel's `SupplierIncidentSource` (07 Manufacturing
//! §7.1 *NCR* cause paragraph, `STK.SUP.01`). The stored monthly snapshot is
//! wl-inventory's and is checked with the real scorecard in
//! `host/tests/manufacturing_incidents.rs`.

mod support;

use manufacturing::{Cause, Plant};
use renox::prelude::*;
use support::{Booted, boot, date};
use wl_kernel::PartyId;
use wl_kernel::inventory::supplier_incidents;

const SUPPLIER: i64 = 7;

async fn scene() -> (Booted, Plant, User) {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let qc = t.inspector("Ines", &plant).await;
    t.lots.put(77, "L-77", Some(SUPPLIER), false, 100_000, 3_000, "quarantine");
    t.lots.put(78, "F-78", None, true, 100_000, 3_000, "quarantine");
    (t, plant, qc)
}

async fn february(t: &Booted) -> Vec<wl_kernel::inventory::SupplierIncident> {
    supplier_incidents(t.state())
        .incidents(t.db(), PartyId(SUPPLIER), date(2027, 2, 1), date(2027, 2, 28))
        .await
        .unwrap()
}

/// AC1: an incoming-inspection NCR on lot L-77 from supplier S raised on
/// 2027-02-10 is one incident dated that day for S and February, however
/// late it is dispositioned or reopened; the other months have none.
#[renox::test]
async fn an_incoming_ncr_is_one_incident_on_the_day_it_was_raised() {
    let (t, plant, qc) = scene().await;
    let ncr = t
        .at(date(2027, 2, 10), 9, 0, t.ncr(&qc, &plant, 77, "10"))
        .await;

    t.at(date(2027, 3, 4), 9, 0, async {
        t.ncr_act(&qc, &ncr, "disposition", &[("disposition", "scrap")])
            .await
            .assert_status(303);
    })
    .await;

    let found = february(&t).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].on, date(2027, 2, 10));
    assert_eq!(found[0].supplier, PartyId(SUPPLIER));
    assert_eq!(found[0].reference, ncr.subject());
    let march = supplier_incidents(t.state())
        .incidents(t.db(), PartyId(SUPPLIER), date(2027, 3, 1), date(2027, 3, 31))
        .await
        .unwrap();
    assert!(march.is_empty(), "the disposition date does not count");
    let other = supplier_incidents(t.state())
        .incidents(t.db(), PartyId(8), date(2027, 2, 1), date(2027, 2, 28))
        .await
        .unwrap();
    assert!(other.is_empty());
}

/// AC2: QC changes the cause to *internal process* with a reason: the adapter
/// no longer returns the NCR; changed back to *supplier* it returns it again.
#[renox::test]
async fn a_changed_cause_drops_the_incident() {
    let (t, plant, qc) = scene().await;
    let ncr = t
        .at(date(2027, 2, 10), 9, 0, t.ncr(&qc, &plant, 77, "10"))
        .await;
    assert_eq!(february(&t).await.len(), 1);

    t.ncr_act(
        &qc,
        &ncr,
        "cause",
        &[("cause", "internal_process"), ("reason", "dried too long")],
    )
    .await
    .assert_status(303);

    assert_eq!(t.ncr_now(ncr.id).await.cause, Cause::InternalProcess);
    assert!(february(&t).await.is_empty());
    t.ncr_act(
        &qc,
        &ncr,
        "cause",
        &[("cause", "supplier"), ("reason", "the lab says supplier")],
    )
    .await
    .assert_status(303);
    assert_eq!(february(&t).await.len(), 1);
}

/// AC3: a cancelled NCR is not returned. AC4: an NCR on a lot from own
/// production has no supplier and is never returned; an *unknown* cause is not
/// returned either.
#[renox::test]
async fn cancelled_own_production_and_unknown_cause_are_not_incidents() {
    let (t, plant, qc) = scene().await;
    let cancelled = t
        .at(date(2027, 2, 10), 9, 0, t.ncr(&qc, &plant, 77, "10"))
        .await;
    let own = t
        .at(date(2027, 2, 11), 9, 0, t.ncr(&qc, &plant, 78, "10"))
        .await;
    let unknown = t
        .at(date(2027, 2, 12), 9, 0, t.ncr(&qc, &plant, 77, "5"))
        .await;
    t.ncr_act(&qc, &unknown, "cause", &[("cause", "unknown"), ("reason", "not sure")])
        .await
        .assert_status(303);

    t.ncr_act(&qc, &cancelled, "cancel", &[("reason", "raised twice")])
        .await
        .assert_status(303);

    assert_eq!(own.supplier_party_id, None);
    assert!(february(&t).await.is_empty());
}
