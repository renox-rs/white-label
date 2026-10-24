//! The dispatch screens of BL-STK-22 (#241): the buttons that confirm a
//! courier's delivery and record the refused goods that came back, a disabled
//! button with its reason for an own delivery, and the delivery note.
//! Capability STK.DSP.01.

mod support;

use wl_inventory::NewDispatchLine;

use support::{Booted, ORDER, boot};

/// Two released lots of a pump in a store and a picked dispatch of 10 from
/// the first.
async fn picked(t: &Booted) -> (wl_inventory::Dispatch, wl_inventory::Lot) {
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", true, 5_000).await;
    let first = t.lot_expiring(&pump, "L9", 30).await;
    let second = t.lot_expiring(&pump, "L10", 60).await;
    t.stock_in(&pump, &store, Some(&first), 10_000, 5_000).await;
    t.stock_in(&pump, &store, Some(&second), 10_000, 5_000)
        .await;
    let dispatcher = t.dispatcher("Dina").await;
    let dispatch = t
        .picked_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000).lot(first.id)],
        )
        .await;
    (dispatch, first)
}

/// The visible text of a page: tags dropped.
fn visible(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    text.replace("&#39;", "'").replace("&amp;", "&")
}

/// A courier dispatch offers Confirm delivery; an own delivery shows it
/// disabled with its reason; after a delivery with a refusal the page shows
/// the refused goods in return and offers to record them back, and the
/// delivery note is served as a PDF.
#[renox::test]
async fn confirming_a_delivery_and_taking_the_refused_goods_back_are_buttons() {
    let t = boot().await;
    let (picked, _lot) = picked(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "courier"), ("tracking_number", "TRK-1")],
    )
    .await
    .assert_ok();
    let path = format!("/dispatches/{}", picked.id);

    let page = t.browse(&dispatcher, &path).await;
    page.assert_ok();
    let text = visible(&page.text());
    assert!(
        text.contains("Confirm delivery") && text.contains("TRK-1"),
        "{text}"
    );
    assert!(page.text().contains("confirm-dispatch"));
    assert!(
        !page.text().contains("refused-back"),
        "nothing was refused yet"
    );

    let line = t.dispatch_lines(&picked).await.remove(0);
    let (delivered, refused, reason) = (
        format!("delivered_{}", line.id),
        format!("refused_{}", line.id),
        format!("reason_{}", line.id),
    );
    t.dispatch_action(
        &dispatcher,
        &picked,
        "confirm",
        &[
            (delivered.as_str(), "8"),
            (refused.as_str(), "2"),
            (reason.as_str(), "damaged"),
        ],
    )
    .await
    .assert_ok();
    let page = t.browse(&dispatcher, &path).await;
    page.assert_ok();
    let text = visible(&page.text());
    for word in [
        "Delivered",
        "Damaged",
        "2 pcs refused goods are still with the driver",
    ] {
        assert!(text.contains(word), "{word} missing in {text}");
    }
    assert!(page.text().contains("refused-back"));

    let note = {
        t.app.acting_as(&dispatcher);
        t.app
            .request()
            .get(&format!("/dispatches/{}/delivery-note", picked.id))
            .await
    };
    note.assert_ok();
    assert!(note.text().starts_with("%PDF"));
}

/// An own delivery cannot be confirmed by hand: the button is disabled and
/// the page says why.
#[renox::test]
async fn an_own_delivery_shows_confirm_disabled_with_its_reason() {
    let t = boot().await;
    let (picked, _lot) = picked(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "own_delivery")],
    )
    .await
    .assert_ok();

    let page = t
        .browse(&dispatcher, &format!("/dispatches/{}", picked.id))
        .await;
    page.assert_ok();
    assert!(!page.text().contains("confirm-dispatch"));
    assert!(page.text().contains("disabled"));
    assert!(
        visible(&page.text()).contains("confirmed by the driver's proof of delivery"),
        "the reason is on the page"
    );
}
