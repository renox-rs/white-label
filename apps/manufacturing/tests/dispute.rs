//! BL-MFG-13 (#394): a quality dispute on an invoice becomes NCRs (07
//! Manufacturing §4.1 row "An invoice is disputed", §4.5 step 5). wl-invoicing
//! announces `InvoiceDisputed` with the kind, the reason and the disputed
//! lines (each line leads to its dispatch line); the vertical raises or links
//! one open NCR per lot, links the dispute and its ticket, and tells QC. The
//! real invoicing, the lines ticked in the portal and the buyer's view are in
//! `host/tests/manufacturing_dispute.rs`.

mod support;

use manufacturing::disputes::links_of;
use manufacturing::{Cause, Ncr, NcrStatus, Origin, Plant};
use renox::prelude::*;
use support::{Booted, boot};
use wl_kernel::invoicing::{DisputeBy, DisputedLine, InvoiceDisputed, InvoiceId};
use wl_kernel::tickets::TicketId;
use wl_kernel::{PartyId, Subject};

fn line(position: i32, dispatch_line: i64) -> DisputedLine {
    DisputedLine {
        position,
        amount: 10_000,
        origin: Some(Subject::new("inventory.dispatch_line", dispatch_line)),
    }
}

async fn dispute(t: &Booted, id: i64, kind: &str, lines: Vec<DisputedLine>) {
    t.state()
        .emit(InvoiceDisputed {
            invoice: InvoiceId(60),
            party: PartyId(21),
            source: Subject::new("manufacturing.dispatch", 1),
            dispute: id,
            amount: 30_000,
            lines,
            ticket: Some(TicketId(5)),
            raised_by: DisputeBy::Party { id: PartyId(21) },
            reason: "colour off".into(),
            kind: Some(kind.into()),
        })
        .await
        .unwrap();
}

async fn all(t: &Booted) -> Vec<Ncr> {
    Ncr::query().order_by(Ncr::ID).get(t.db()).await.unwrap()
}

async fn scene() -> (Booted, Plant, User) {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let qc = t.inspector("Ines", &plant).await;
    for (id, number) in [(3, "F3"), (4, "F4"), (5, "F5"), (6, "F6")] {
        t.lots.put(id, number, None, true, 0, 2_000, "released");
    }
    t.dispatches.put(1, &[(Some(3), 10_000)]);
    t.dispatches.put(2, &[(Some(4), 5_000), (Some(5), 7_000)]);
    t.dispatches.put(3, &[(Some(6), 8_000)]);
    t.dispatches.put(4, &[(None, 2_000)]);
    (t, plant, qc)
}

/// AC1: INV-60 for lot F3, the buyer disputes it for *quality* with "colour
/// off": an open NCR on F3 with the text exists, linked to the dispute and its
/// ticket, raised by the system with the origin *dispute* and the cause
/// *internal process* (the lot is the plant's), and QC is notified.
#[renox::test]
async fn a_quality_dispute_raises_an_ncr_on_the_lot_and_tells_qc() {
    let (t, _, qc) = scene().await;

    dispute(&t, 7, "quality", vec![line(1, 1)]).await;

    let ncrs = all(&t).await;
    assert_eq!(ncrs.len(), 1);
    assert_eq!(ncrs[0].lot_number.as_deref(), Some("F3"));
    assert_eq!(ncrs[0].defect, "colour off");
    assert_eq!(ncrs[0].status, NcrStatus::Open);
    assert_eq!(ncrs[0].origin, Origin::Dispute);
    assert_eq!(ncrs[0].cause, Cause::InternalProcess);
    assert_eq!(ncrs[0].quantity_milli, 10_000);
    assert_eq!(ncrs[0].raised_by, None);
    let links = links_of(t.db(), ncrs[0].id).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].dispute_id, 7);
    assert_eq!(links[0].invoice_id, 60);
    assert_eq!(links[0].ticket_id, Some(5));
    let told = t.notices.titles_for(&qc);
    assert_eq!(told.len(), 1);
    assert!(told[0].starts_with("Quality dispute: NCR-"), "{told:?}");
}

/// AC2: INV-61 with line 1 (dispatch line with lots F4 and F5) and line 2
/// (lot F6): ticking line 1 only gives NCRs on F4 and F5 and none on F6; with
/// both lines (what the portal sends when none is ticked) F6 has one too.
#[renox::test]
async fn the_ticked_lines_decide_which_lots_get_an_ncr() {
    let (t, _, _) = scene().await;

    dispute(&t, 8, "quality", vec![line(1, 2)]).await;
    let first: Vec<_> = all(&t).await.iter().map(|n| n.lot_id).collect();
    dispute(&t, 9, "quality", vec![line(1, 2), line(2, 3)]).await;

    assert_eq!(first, [Some(4), Some(5)]);
    let lots: Vec<_> = all(&t).await.iter().map(|n| n.lot_id).collect();
    assert_eq!(
        lots,
        [Some(4), Some(5), Some(6)],
        "F4 and F5 are linked again, not raised twice"
    );
    let f4 = &all(&t).await[0];
    assert_eq!(links_of(t.db(), f4.id).await.unwrap().len(), 2);
}

/// AC3: a dispute about the price creates no NCR; neither does a line that
/// bills no dispatch.
#[renox::test]
async fn a_price_dispute_creates_no_ncr() {
    let (t, _, _) = scene().await;

    dispute(&t, 10, "price", vec![line(1, 1)]).await;
    dispute(
        &t,
        11,
        "quality",
        vec![DisputedLine {
            position: 1,
            amount: 1,
            origin: None,
        }],
    )
    .await;

    assert!(all(&t).await.is_empty());
}

/// A lot on two disputed lines gets one NCR; an item with no lot gets one on
/// the item and the dispatch line; a repeat of the same event creates nothing
/// more.
#[renox::test]
async fn one_ncr_per_lot_and_one_on_the_item_when_it_has_no_lot() {
    let (t, _, _) = scene().await;
    t.dispatches.put(5, &[(Some(3), 4_000)]);

    dispute(&t, 12, "quality", vec![line(1, 1), line(2, 5), line(3, 4)]).await;
    dispute(&t, 12, "quality", vec![line(1, 1), line(2, 5), line(3, 4)]).await;

    let ncrs = all(&t).await;
    assert_eq!(ncrs.len(), 2);
    assert_eq!(ncrs[0].lot_id, Some(3));
    assert_eq!(ncrs[1].lot_id, None);
    assert_eq!(ncrs[1].item_id, support::RESIN);
    assert_eq!(
        ncrs[1].source(),
        Some(Subject::new("inventory.dispatch_line", 4))
    );
    assert_eq!(links_of(t.db(), ncrs[0].id).await.unwrap().len(), 1);
}
