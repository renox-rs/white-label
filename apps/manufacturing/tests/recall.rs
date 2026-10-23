//! BL-MFG-10 (#391): a recalled lot traced to the customers (07 §4.4 step 8,
//! §9 *Traceability and recall*): the app's `TracePartyResolver` says whose a
//! dispatch or a work order is, a dispatch that has not left is flagged for a
//! re-pick with a task for the storekeeper, one that left is listed for the
//! recall owner to bring back, and the notice goes to the customer's buyers or
//! its main contact.

mod support;

use manufacturing::recalls::{self, ProductionParties, RepickFlag, repick_key};
use manufacturing::{WorkOrder, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;
use support::{Booted, boot, date, holder};
use wl_kernel::inventory::{
    DELIVERED, NOT_SHIPPED, RecallTraced, SHIPPED, TraceParty, TracePartyResolver,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::{PartyId, Subject};

fn dispatch(id: i64) -> Subject {
    Subject::new("inventory.dispatch", id)
}

/// A work order of the plant serving line `line` of sales order `order`,
/// `in_progress` (a fixture).
async fn for_order(t: &Booted, order: i64, line: i64) -> WorkOrder {
    let plant = t.plant("Plant 1").await;
    let floor = t.line(&plant, "L1").await;
    let made = t.fixture(&plant, &floor, WorkOrderStatus::InProgress).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(made.id))
        .update(
            t.db(),
            &[
                ("source", &WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(order)),
                ("sales_order_line_id", &Some(line)),
            ],
        )
        .await
        .unwrap();
    t.reload(made.id).await
}

/// AC1: DN-1 (left, customer C1) is C1 *shipped*; DN-2 (`picked`, customer
/// C2) is C2 *not shipped*; a delivered one is *delivered* (shown as shipped).
#[renox::test]
async fn a_dispatch_is_its_customer_shipped_or_not() {
    let t = boot().await;
    t.dispatches
        .directory(1, "dispatched", 80, Some(101), Some(date(2026, 10, 2)));
    t.dispatches.directory(2, "picked", 81, Some(102), None);
    t.dispatches
        .directory(3, "delivered", 82, Some(103), Some(date(2026, 10, 1)));

    let parties = ProductionParties;
    let state = t.state();
    assert_eq!(
        parties.party(state, &dispatch(1)).await.unwrap(),
        TraceParty::Party(PartyId(101))
    );
    assert_eq!(
        parties
            .delivery_status(state, &dispatch(1))
            .await
            .unwrap()
            .as_deref(),
        Some(SHIPPED)
    );
    assert_eq!(
        parties.party(state, &dispatch(2)).await.unwrap(),
        TraceParty::Party(PartyId(102))
    );
    assert_eq!(
        parties
            .delivery_status(state, &dispatch(2))
            .await
            .unwrap()
            .as_deref(),
        Some(NOT_SHIPPED)
    );
    assert_eq!(
        parties
            .delivery_status(state, &dispatch(3))
            .await
            .unwrap()
            .as_deref(),
        Some(DELIVERED)
    );
}

/// A returned or cancelled dispatch is not listed (the goods are back), and a
/// dispatch nobody knows has no party.
#[renox::test]
async fn a_returned_or_cancelled_dispatch_is_not_affected() {
    let t = boot().await;
    t.dispatches
        .directory(4, "returned", 83, Some(101), Some(date(2026, 10, 2)));
    t.dispatches.directory(5, "cancelled", 84, Some(101), None);
    t.dispatches.directory(6, "picked", 85, None, None);

    for id in [4, 5] {
        assert_eq!(
            ProductionParties
                .party(t.state(), &dispatch(id))
                .await
                .unwrap(),
            TraceParty::NotAffected
        );
    }
    assert_eq!(
        ProductionParties
            .party(t.state(), &dispatch(6))
            .await
            .unwrap(),
        TraceParty::Unknown,
        "a dispatch with no recipient has no party"
    );
    assert_eq!(
        ProductionParties
            .party(t.state(), &dispatch(99))
            .await
            .unwrap(),
        TraceParty::Unknown
    );
}

/// AC2: WO-701 `in_progress` for SO-80 of customer C3 is C3 *not shipped*.
/// AC3: a work order for stock has no party; an order whose line is
/// cancelled or has nothing open any more has none either.
#[renox::test]
async fn a_work_order_is_the_customer_of_an_order_not_yet_delivered() {
    let t = boot().await;
    t.lines.open(80, 1, 1_000_000, None);
    t.lines.customer(80, 103);
    let linked = for_order(&t, 80, 1).await;
    let subject = linked.subject();

    assert_eq!(
        ProductionParties.party(t.state(), &subject).await.unwrap(),
        TraceParty::Party(PartyId(103))
    );
    assert_eq!(
        ProductionParties
            .delivery_status(t.state(), &subject)
            .await
            .unwrap()
            .as_deref(),
        Some(NOT_SHIPPED)
    );

    // For stock: no party.
    let plant = t.plant("Plant 2").await;
    let line = t.line(&plant, "L2").await;
    let stock = t.fixture(&plant, &line, WorkOrderStatus::InProgress).await;
    assert_eq!(
        ProductionParties
            .party(t.state(), &stock.subject())
            .await
            .unwrap(),
        TraceParty::Unknown
    );

    // The order was delivered meanwhile: nothing is open on its line.
    t.lines.cancel(80, 1);
    assert_eq!(
        ProductionParties.party(t.state(), &subject).await.unwrap(),
        TraceParty::Unknown
    );
}

/// AC4: the notice goes to every active portal buyer of the customer; a
/// customer with none is told through its own contact points (its main
/// contact email).
#[renox::test]
async fn the_notice_goes_to_the_buyers_or_the_main_contact() {
    let t = boot().await;
    t.buyers.add(101, 7);
    t.buyers.add(101, 8);

    let with_buyers = ProductionParties
        .addressees(t.state(), PartyId(101))
        .await
        .unwrap();
    let without = ProductionParties
        .addressees(t.state(), PartyId(104))
        .await
        .unwrap();

    assert_eq!(
        with_buyers,
        vec![
            NotifyRecipient::User { id: 7 },
            NotifyRecipient::User { id: 8 }
        ]
    );
    assert_eq!(without, vec![NotifyRecipient::Party { id: PartyId(104) }]);
}

/// AC1: DN-2 (`picked`) holding a recalled lot is flagged for a re-pick with
/// one task for the storekeeper, once for the recall even when the trace runs
/// again; DN-1 (left) gets no task and is on the list to bring back.
#[renox::test]
async fn a_dispatch_that_has_not_left_is_flagged_for_a_repick_once() {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let keeper = t.keeper("Karim", &plant).await;
    t.dispatches
        .directory(1, "dispatched", 80, Some(101), Some(date(2026, 10, 2)));
    t.dispatches.directory(2, "picked", 81, Some(102), None);
    let lines = vec![
        holder(1, dispatch(1), Some(101), 31, SHIPPED),
        holder(2, dispatch(2), Some(102), 31, NOT_SHIPPED),
    ];
    t.recalls.put(5, lines.clone());
    let event = RecallTraced {
        recall: Subject::new("inventory.recall", 5),
        number: "RCL-5".into(),
        lots: vec![Subject::new("inventory.lot", 31)],
        lines,
    };

    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    let flags = RepickFlag::query().get(t.db()).await.unwrap();
    assert_eq!(
        flags.len(),
        1,
        "one flag for the dispatch that has not left"
    );
    assert_eq!((flags[0].dispatch_id, flags[0].recall_id), (2, 5));
    let tasks = t.tickets.all();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].ticket.source_key.as_deref(),
        Some(repick_key(5, 2).as_str())
    );
    assert_eq!(tasks[0].ticket.subject, dispatch(2));
    assert_eq!(
        tasks[0].ticket.assignee,
        Some(wl_kernel::tickets::Assignee::user(keeper.id))
    );

    let rows = recalls::trace(t.state(), 5).await.unwrap();
    let left = rows.iter().find(|row| row.label == "DSP-1").unwrap();
    let held = rows.iter().find(|row| row.label == "DSP-2").unwrap();
    assert!(left.shipped && left.bring_back && !left.repick);
    assert!(!held.shipped && !held.bring_back && held.repick);
}
