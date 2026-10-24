//! BL-MFG-06 (#387): the buyer's PO upload in the portal refuses a customer
//! PO number an order of the customer holds already (07 §4.1, §4.5 step 2):
//! trimmed and case-insensitive, with its call-off number, a cancelled order
//! included, naming the number and status of the order only.

mod support;

use manufacturing::po_upload::PoUpload;
use renox::prelude::*;
use support::{Booted, boot};

const C: i64 = 501;

/// SO-70 of C holds "PO-8812" and was cancelled SO-72 holds "PO-9000";
/// SO-71 holds blanket "BL-1" call-off "CO-1". Buyer B acts for C.
async fn scene() -> (Booted, User) {
    let t = boot().await;
    let buyer = t.user("Bilal").await;
    t.buyers.add(C, buyer.id);
    t.po_orders.hold(C, "PO-8812", "", "SO-70", "Confirmed");
    t.po_orders.hold(C, "PO-9000", "", "SO-72", "Cancelled");
    t.po_orders.hold(C, "BL-1", "CO-1", "SO-71", "Confirmed");
    (t, buyer)
}

async fn upload(t: &Booted, by: &User, po: &str, call_off: &str) -> renox::testing::TestResponse {
    t.post(
        by,
        "/portal/manufacturing/orders",
        &[("po", po), ("call_off", call_off), ("note", "by email too")],
    )
    .await
}

/// AC3 and AC6: "po-8812 " is refused and the buyer sees SO-70 and its status,
/// nothing else; a cancelled order still holds its number; "PO 8812" is a
/// different number and is accepted.
#[renox::test]
async fn a_duplicate_upload_is_refused_naming_the_order_and_its_status() {
    let (t, buyer) = scene().await;

    let refused = upload(&t, &buyer, "po-8812 ", "").await;
    refused.assert_status(422);
    let text = refused.text();
    assert!(
        text.contains("SO-70") && text.contains("Confirmed"),
        "{text}"
    );
    assert!(!text.contains("by email too"));

    let cancelled = upload(&t, &buyer, "PO-9000", "").await;
    cancelled.assert_status(422);
    assert!(cancelled.text().contains("SO-72"), "{}", cancelled.text());
    assert!(cancelled.text().contains("Cancelled"));

    upload(&t, &buyer, "PO 8812", "").await.assert_status(303);
    assert_eq!(PoUpload::query().count(t.db()).await.unwrap(), 1);
}

/// AC4: with "BL-1" / "CO-1" held, "BL-1" / "CO-2" is accepted and
/// "BL-1" / "CO-1" is refused; the same upload twice keeps one.
#[renox::test]
async fn a_new_call_off_number_is_accepted_and_the_same_one_is_not() {
    let (t, buyer) = scene().await;

    upload(&t, &buyer, "BL-1", "CO-2").await.assert_status(303);
    upload(&t, &buyer, "BL-1", "CO-1").await.assert_status(422);
    let twice = upload(&t, &buyer, "BL-1", "co-2 ").await;
    twice.assert_status(422);

    assert_eq!(PoUpload::query().count(t.db()).await.unwrap(), 1);
    let page = t.browse(&buyer, "/portal/manufacturing/orders").await;
    page.assert_ok();
    assert!(page.text().contains("BL-1") && page.text().contains("CO-2"));
}

/// AC5: the same number for another customer is accepted, and the buyer sees
/// only the uploads of the company they act for; somebody who acts for no
/// company is refused.
#[renox::test]
async fn another_customer_may_use_the_number_and_a_stranger_is_refused() {
    let (t, buyer) = scene().await;
    let other = t.user("Dana").await;
    t.buyers.add(502, other.id);
    let stranger = t.user("Sam").await;

    upload(&t, &other, "PO-8812", "").await.assert_status(303);
    upload(&t, &stranger, "PO-1", "").await.assert_status(403);

    let page = t.browse(&buyer, "/portal/manufacturing/orders").await;
    page.assert_ok();
    assert!(!page.text().contains("PO-8812"));
    assert!(!page.text().contains("None"));
}
