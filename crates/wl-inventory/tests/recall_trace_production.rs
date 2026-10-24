//! The recall trace through a production source and a dispatch that has not
//! left (BL-MFG-10, #391; wl-inventory §4 *Recall* forward trace, §6
//! `TracePartyResolver`): a work order the vertical knows the party of is a
//! line of its own, a `draft` or `picked` dispatch holding a recalled lot is
//! listed with the delivery status the vertical gives, a returned dispatch is
//! not, and the notice goes to the recipients the vertical names.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::{Item, Location, Lot, LotStatus, NewDispatchLine, Recall, RecallLine};
use wl_kernel::inventory::{NOT_SHIPPED, SHIPPED};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

fn at(day: u32, hour: u32) -> DateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, 0, 0)
        .unwrap()
        .and_utc()
}

fn work_order(n: i64) -> Subject {
    Subject::new("manufacturing.work_order", n)
}

/// Raw lot R1 issued to a work order that made lot F1 (10 pieces) at the
/// finished goods store; returns the items, the lots and the store.
struct Plant {
    resin: Item,
    panel: Item,
    r1: Lot,
    f1: Lot,
    rm: Location,
    fg: Location,
}

async fn plant(t: &Booted) -> Plant {
    let resin = t.tracked_item("RESIN").await;
    let panel = t.tracked_item("PANEL").await;
    let r1 = t.lot(&resin, "R1", LotStatus::Released).await;
    let f1 = t.lot(&panel, "F1", LotStatus::Released).await;
    let rm = t.named_location("RM", "Raw store").await;
    let fg = t.named_location("FG", "Finished goods").await;
    t.stock_in(&resin, &rm, Some(&r1), 100_000, 800).await;
    Plant {
        resin,
        panel,
        r1,
        f1,
        rm,
        fg,
    }
}

/// Work order `n` issues `quantity` of R1 and makes `made` of F1 at the day.
async fn produced(t: &Booted, p: &Plant, n: i64, quantity: i64, made: i64, day: u32) {
    let order = work_order(n);
    t.movement(
        MovementType::Consume,
        &p.resin,
        Some(&p.r1),
        Some(&p.rm),
        None,
        quantity,
        &order,
        Some(&order),
        at(day, 8),
        false,
    )
    .await;
    t.movement(
        MovementType::Produce,
        &p.panel,
        Some(&p.f1),
        None,
        Some(&p.fg),
        made,
        &order,
        None,
        at(day, 9),
        false,
    )
    .await;
}

async fn recall_r1(t: &Booted, p: &Plant) -> Recall {
    let manager = t.recaller("Rita").await;
    let item_id = p.resin.id.to_string();
    t.recall_as(
        &manager,
        &[
            ("source", "supplier"),
            ("reason", "Contaminated resin"),
            ("items", item_id.as_str()),
            ("lot_numbers", "R1"),
        ],
    )
    .await
    .assert_ok();
    t.all_recalls().await.pop().expect("a recall")
}

async fn lines(t: &Booted, recall: &Recall) -> Vec<RecallLine> {
    t.recall_lines(recall).await
}

/// BL-MFG-10 AC1: raw lot R1 issued to WO-700 producing F1, F1 dispatched on
/// DN-1 to customer C1 and reserved on DN-2 (`picked`) for C2: a line for C1
/// *shipped* and one for C2 *not shipped*, on the dispatches.
#[renox::test]
async fn a_dispatch_that_left_is_shipped_and_a_picked_one_is_not_shipped() {
    let t = boot().await;
    let p = plant(&t).await;
    produced(&t, &p, 700, 40_000, 10_000, 2).await;
    let store = p.fg.clone();
    let keeper = t.dispatcher("Dina").await;
    let dn1 = t
        .sent_dispatch(
            &keeper,
            &store,
            80,
            vec![NewDispatchLine::of(p.panel.id, 4_000).lot(p.f1.id)],
        )
        .await;
    let dn2 = t
        .picked_dispatch(
            &keeper,
            &store,
            81,
            vec![NewDispatchLine::of(p.panel.id, 3_000).lot(p.f1.id)],
        )
        .await;
    t.trace_parties.set(&dn1.subject(), PartyId(101));
    t.trace_parties.delivery(&dn1.subject(), SHIPPED);
    t.trace_parties.set(&dn2.subject(), PartyId(102));
    t.trace_parties.delivery(&dn2.subject(), NOT_SHIPPED);

    let recall = recall_r1(&t, &p).await;

    let found = lines(&t, &recall).await;
    let of = |dispatch: &wl_inventory::Dispatch| {
        found
            .iter()
            .find(|line| line.used_by() == Some(dispatch.subject()))
            .cloned()
            .expect("a line for the dispatch")
    };
    let first = of(&dn1);
    assert_eq!(
        (first.party_id, first.delivery_status.as_deref()),
        (Some(101), Some(SHIPPED))
    );
    assert_eq!(first.kind.as_str(), "dispatch");
    let second = of(&dn2);
    assert_eq!(
        (second.party_id, second.delivery_status.as_deref()),
        (Some(102), Some(NOT_SHIPPED))
    );
    assert_eq!(second.lot_id, Some(p.f1.id));
}

/// BL-MFG-10 AC2: WO-701 for the order of customer C3 has a line for C3 *not
/// shipped*; AC3: WO-702 for stock has no line and its output lot is in scope.
#[renox::test]
async fn a_work_order_for_an_order_is_a_line_of_its_customer_and_one_for_stock_is_none() {
    let t = boot().await;
    let p = plant(&t).await;
    produced(&t, &p, 701, 30_000, 5_000, 2).await;
    produced(&t, &p, 702, 20_000, 5_000, 3).await;
    t.trace_parties.set(&work_order(701), PartyId(103));
    t.trace_parties.delivery(&work_order(701), NOT_SHIPPED);

    let recall = recall_r1(&t, &p).await;

    let found = lines(&t, &recall).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind.as_str(), "production");
    assert_eq!(found[0].used_by(), Some(work_order(701)));
    assert_eq!(
        (found[0].party_id, found[0].delivery_status.as_deref()),
        (Some(103), Some(NOT_SHIPPED))
    );
    let scope = wl_inventory::recall::lots_in_scope(t.db(), recall.id)
        .await
        .unwrap();
    assert_eq!(scope, vec![p.r1.id, p.f1.id]);
}

/// BL-MFG-10: a returned or cancelled dispatch is not listed (the goods are
/// back on site).
#[renox::test]
async fn a_dispatch_the_vertical_says_is_back_is_not_listed() {
    let t = boot().await;
    let p = plant(&t).await;
    produced(&t, &p, 700, 40_000, 10_000, 2).await;
    let keeper = t.dispatcher("Dina").await;
    let back = t
        .sent_dispatch(
            &keeper,
            &p.fg,
            82,
            vec![NewDispatchLine::of(p.panel.id, 2_000).lot(p.f1.id)],
        )
        .await;
    t.trace_parties.not_affected(&back.subject());

    let recall = recall_r1(&t, &p).await;

    assert!(lines(&t, &recall).await.is_empty());
}

/// BL-MFG-10: a line keeps its place when the trace runs again and takes the
/// delivery status the vertical gives now.
#[renox::test]
async fn tracing_again_updates_the_delivery_status_of_a_line() {
    let t = boot().await;
    let p = plant(&t).await;
    produced(&t, &p, 700, 40_000, 10_000, 2).await;
    let keeper = t.dispatcher("Dina").await;
    let dn = t
        .picked_dispatch(
            &keeper,
            &p.fg,
            83,
            vec![NewDispatchLine::of(p.panel.id, 3_000).lot(p.f1.id)],
        )
        .await;
    t.trace_parties.set(&dn.subject(), PartyId(104));
    t.trace_parties.delivery(&dn.subject(), NOT_SHIPPED);
    let recall = recall_r1(&t, &p).await;
    assert_eq!(
        lines(&t, &recall).await[0].delivery_status.as_deref(),
        Some(NOT_SHIPPED)
    );

    t.trace_parties.delivery(&dn.subject(), SHIPPED);
    let manager = t.recaller("Rita").await;
    t.trace_as(&manager, &recall).await.assert_ok();

    let found = lines(&t, &recall).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].delivery_status.as_deref(), Some(SHIPPED));
}

/// BL-MFG-10 AC4: the notice goes to the recipients the vertical names for
/// the party: the buyers, else the party itself.
#[renox::test]
async fn the_notice_goes_to_the_recipients_the_vertical_names() {
    let t = boot().await;
    let p = plant(&t).await;
    produced(&t, &p, 701, 30_000, 5_000, 2).await;
    t.trace_parties.set(&work_order(701), PartyId(103));
    t.trace_parties.addressed(
        PartyId(103),
        vec![
            NotifyRecipient::User { id: 7 },
            NotifyRecipient::User { id: 8 },
        ],
    );
    let recall = recall_r1(&t, &p).await;
    let line = lines(&t, &recall).await.remove(0);
    let manager = t.recaller("Rita").await;

    t.notices_as(&manager, &recall, "customer", &[&line])
        .await
        .assert_ok();

    let sent = t
        .notices
        .all()
        .into_iter()
        .find(|request| request.type_key.ends_with("recall_notice_customer"))
        .expect("a notice");
    assert_eq!(
        sent.recipients,
        vec![
            NotifyRecipient::User { id: 7 },
            NotifyRecipient::User { id: 8 }
        ]
    );
}
