//! The dispatch screens of BL-STK-21 (#240): the list, the record page with
//! its lines and buttons, a lot on hold shown with the re-pick button, a
//! disabled button with its reason, and the refusal of a failed credit
//! re-check in the sheet. Capability STK.DSP.01.

mod support;

use wl_inventory::NewDispatchLine;
use wl_kernel::sales::CreditFailure;

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

/// The list names the dispatch, its recipient, its source and its status; a
/// browser gets the page, other clients JSON; a user who may neither manage
/// nor see stock is refused.
#[renox::test]
async fn the_list_shows_the_dispatches_with_their_status() {
    let t = boot().await;
    let (dispatch, _lot) = picked(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    let nobody = t.clerk("Carl").await;

    let page = t.browse(&dispatcher, "/dispatches").await;
    page.assert_ok();
    let text = visible(&page.text());
    for word in [
        "Dispatches",
        dispatch.number.as_deref().unwrap(),
        "Al Noor Trading",
        "Picked",
        "PUMP",
    ] {
        assert!(text.contains(word), "{word} missing in {text}");
    }
    assert!(
        page.text()
            .contains(&format!("/dispatches/{}", dispatch.id))
    );
    t.browse(&nobody, "/dispatches").await.assert_forbidden();

    t.app.acting_as(&dispatcher);
    let json = t.app.request().get("/dispatches").await;
    json.assert_ok();
    assert!(
        json.text().contains("\"status_code\":\"picked\""),
        "{}",
        json.text()
    );
}

/// A picked dispatch shows its lines and the buttons that dispatch, cancel
/// and re-pick; once a picked lot is held the page says so with the line's
/// lot, offers the re-pick and shows Dispatch disabled with its reason; a
/// viewer without `dispatch.manage` sees no button.
#[renox::test]
async fn a_picked_dispatch_with_a_held_lot_asks_for_a_repick() {
    let t = boot().await;
    let (dispatch, lot) = picked(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    let viewer = t.finance("Fiona").await;
    let path = format!("/dispatches/{}", dispatch.id);

    let page = t.browse(&dispatcher, &path).await;
    page.assert_ok();
    let text = visible(&page.text());
    for word in [
        "Picked",
        "Jebel Ali warehouse",
        "PUMP",
        "L9",
        "Re-pick",
        "Cancel",
        "Dispatch",
    ] {
        assert!(text.contains(word), "{word} missing in {text}");
    }
    assert!(
        page.text().contains("send-dispatch"),
        "the dispatch sheet is offered"
    );
    // The lot-less words never leak as keys.
    assert!(!page.text().contains("wl_inventory."));

    t.hold_lot(&lot).await;
    let page = t.browse(&dispatcher, &path).await;
    page.assert_ok();
    let text = visible(&page.text());
    assert!(
        text.contains("Line 1: lot L9 is on hold. Re-pick the line from another released lot"),
        "{text}"
    );
    assert!(
        !page.text().contains("send-dispatch"),
        "Dispatch is disabled while a lot is held"
    );
    assert!(page.text().contains("disabled"));
    assert!(
        page.text().contains("repick-"),
        "the re-pick sheet is offered"
    );

    // A viewer sees the page and no button.
    let page = t.browse(&viewer, &path).await;
    page.assert_ok();
    assert!(!page.text().contains("repick-"));
    assert!(!page.text().contains("cancel-dispatch"));
    assert!(!page.text().contains("pick-dispatch"));
}

/// The refusal of the service shows as the sentence of the guard: a failed
/// credit re-check keeps the dispatch picked.
#[renox::test]
async fn a_failed_credit_recheck_is_a_sentence_in_the_sheet() {
    let t = boot().await;
    let (dispatch, _lot) = picked(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    t.credit.fail(ORDER, CreditFailure::AccountOnHold);

    let refusal = t
        .dispatch_action(
            &dispatcher,
            &dispatch,
            "dispatch",
            &[("handover", "courier"), ("tracking_number", "TRK-1")],
        )
        .await;
    refusal.assert_status(422);
    let text = refusal.text().replace("&#39;", "'");
    assert!(
        text.contains("account is on hold") && text.contains("stays picked"),
        "{text}"
    );
}

/// The pages fit a phone: the list is a grid that turns each row into a
/// card under 768 px and scrolls its table in a region of its own, the
/// record's tables sit in a wrapper that scrolls and the sheets are sheets.
#[renox::test]
async fn the_pages_fit_a_phone() {
    let t = boot().await;
    let (dispatch, _lot) = picked(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    let list = t.browse(&dispatcher, "/dispatches").await.text();
    assert!(
        list.contains("rx-grid--cards"),
        "the rows are cards on a phone"
    );
    assert_eq!(
        list.matches("<table").count(),
        list.matches("rx-grid__scroll").count()
    );
    let page = t
        .browse(&dispatcher, &format!("/dispatches/{}", dispatch.id))
        .await
        .text();
    let tables = page.matches("<table").count();
    assert!(tables > 0);
    assert_eq!(tables, page.matches("rx-table-wrap").count());
    assert!(page.contains("rx-sheet"));
}
