//! BL-MFG-11 (#392) points 6 and 7 on the wl-sales side (`MFG.ACL.11`,
//! `DOC.RUL.02`): an order carries where it is delivered and each line the
//! day it is promised for. Submit and amend ask the kernel `OrderLineRules`
//! about every line and refuse an *export only* item on a domestic delivery
//! with a 422 in words; without a rule provider every line passes; wl-sales
//! provides `DeliveryDestination` and `PromisedLines`. The rule and the stock
//! service are fakes of the vertical and inventory; the orders, the pages,
//! the machine, the amendment and the two providers are the real ones.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{ItemLabel, StockChoice, StockFuture, StockService};
use wl_kernel::sales::{
    OrderLineRules, SalesFuture, delivery_destination, promised_lines, provide_order_line_rules,
};
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use support::Booted;
use support::boot_configured;
use support::changes::{amend, lines, reload};
use wl_sales::permissions as perm;

/// The item the plant sells only abroad.
const LIDS: &str = "LIDS";
/// An item sold anywhere.
const CUPS: &str = "CUPS";

/// The ids of the two items in the fake stock.
fn item_id(code: &str) -> Option<i64> {
    match code {
        LIDS => Some(41),
        CUPS => Some(42),
        _ => None,
    }
}

/// Inventory as sales sees it: it finds an item by its code and by its id.
struct Stock;

impl StockService for Stock {
    fn choices<'a>(
        &'a self,
        _state: &'a AppState,
        code: &'a str,
        _location: &'a Subject,
    ) -> StockFuture<'a, Option<StockChoice>> {
        Box::pin(async move {
            Ok(item_id(code).map(|id| StockChoice {
                item: Subject::new("inventory.item", id),
                lot_tracked: false,
                lots: Vec::new(),
            }))
        })
    }

    fn item_label<'a>(
        &'a self,
        _state: &'a AppState,
        item: &'a Subject,
    ) -> StockFuture<'a, Option<ItemLabel>> {
        Box::pin(async move {
            Ok([LIDS, CUPS]
                .into_iter()
                .find(|code| item_id(code) == Some(item.subject_id))
                .map(|code| ItemLabel {
                    code: code.to_owned(),
                    name: code.to_owned(),
                }))
        })
    }
}

/// The vertical's rule: lids are export only.
struct ExportOnly;

impl OrderLineRules for ExportOnly {
    fn refuse_line<'a>(
        &'a self,
        _state: &'a AppState,
        item: &'a Subject,
        domestic: bool,
    ) -> SalesFuture<'a, Option<String>> {
        Box::pin(async move {
            Ok((domestic && item.subject_id == 41).then(|| {
                "Lids are export only: they cannot be sold for delivery in the UAE.".to_owned()
            }))
        })
    }
}

/// Inventory and, when `rule`, the vertical's line rule.
struct Plant {
    rule: bool,
}

impl Module for Plant {
    fn name(&self) -> &'static str {
        "plant"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::inventory::provide_stock_service(app, Stock);
        if self.rule {
            provide_order_line_rules(app, ExportOnly);
        }
    }
}

async fn boot(rule: bool) -> (Booted, User) {
    let t = boot_configured(WlSales::new(), Plant { rule }).await;
    wl_kernel::approvals::install(t.db()).await.unwrap();
    let sam = t
        .user(
            "Sam",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::SALES_ORDERS_CREATE,
                perm::SALES_ORDERS_ACCEPT,
                perm::SALES_ORDERS_AMEND,
                perm::SALES_ORDERS_CANCEL,
            ],
        )
        .await;
    t.app.acting_as(&sam);
    (t, sam)
}

/// A draft order of customer 7 with a line for each of `codes`, through the
/// pages; `delivery` are the delivery fields of the create form.
async fn draft(t: &Booted, codes: &[&str], delivery: &[(&str, &str)]) -> SalesOrder {
    let mut form = vec![("party_id", "7"), ("channel", "phone")];
    form.extend_from_slice(delivery);
    t.app.post("/sales-orders", &form).await.assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the order was entered");
    for code in codes {
        t.app
            .post(
                &format!("/sales-orders/{}/lines", order.id),
                &[
                    ("item_code", code),
                    ("unit_price", "10.00"),
                    ("quantity", "5"),
                ],
            )
            .await
            .assert_status(303);
    }
    reload(t, order.id).await
}

async fn submit(t: &Booted, order: &SalesOrder) -> renox::testing::TestResponse {
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

/// BL-MFG-11 pt 6 (submit): an *export only* item on an order delivered in
/// the UAE is refused with a 422 in words and the order stays a draft; on an
/// order whose delivery is abroad it is placed; an order that does not say
/// where it goes counts as domestic; another item passes on a domestic order.
#[renox::test]
async fn an_export_only_item_is_refused_on_a_domestic_order_at_submit() {
    let (t, _sam) = boot(true).await;

    let home = draft(&t, &[LIDS, CUPS], &[("delivery_destination", "domestic")]).await;
    let refused = submit(&t, &home).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Lids are export only"),
        "{}",
        refused.text()
    );
    assert!(!refused.text().contains("CUPS"), "only the lids are named");
    assert_eq!(reload(&t, home.id).await.status, SalesOrderStatus::Draft);

    let unsaid = draft(&t, &[LIDS], &[]).await;
    submit(&t, &unsaid).await.assert_status(422);
    assert_eq!(reload(&t, unsaid.id).await.status, SalesOrderStatus::Draft);

    let abroad = draft(
        &t,
        &[LIDS, CUPS],
        &[
            ("delivery_destination", "export"),
            ("delivery_country", "OM"),
        ],
    )
    .await;
    submit(&t, &abroad).await.assert_status(303);
    assert_eq!(
        reload(&t, abroad.id).await.status,
        SalesOrderStatus::Confirmed
    );

    let plain = draft(&t, &[CUPS], &[("delivery_country", "ae")]).await;
    assert_eq!(plain.delivery_destination.as_deref(), Some("domestic"));
    submit(&t, &plain).await.assert_status(303);
    assert_eq!(
        reload(&t, plain.id).await.status,
        SalesOrderStatus::Confirmed
    );

    // Changing the draft's delivery to abroad lets the same order through.
    t.app
        .post(
            &format!("/sales-orders/{}/delivery", home.id),
            &[
                ("delivery_destination", "export"),
                ("delivery_country", "SA"),
                ("delivery_address", "Dammam port, gate 4"),
            ],
        )
        .await
        .assert_status(303);
    submit(&t, &home).await.assert_status(303);
}

/// BL-MFG-11 pt 6 (no provider): without a rule provider the default accepts
/// every line, also on a domestic order.
#[renox::test]
async fn without_a_rule_provider_every_line_passes() {
    let (t, _sam) = boot(false).await;
    let order = draft(&t, &[LIDS], &[("delivery_destination", "domestic")]).await;
    submit(&t, &order).await.assert_status(303);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
}

/// BL-MFG-11 pt 6 (amend): amending a placed order refuses an *export only*
/// line that is added to a domestic order, one enlarged on it, and a change of
/// the delivery from abroad to the UAE while the order holds one; an amendment
/// that adds another item, or moves a domestic order abroad, goes through.
#[renox::test]
async fn an_export_only_item_is_refused_on_a_domestic_order_at_amend() {
    let (t, _sam) = boot(true).await;

    // A domestic order placed before the item was flagged: an old line stays,
    // a plain amendment of another line is not refused.
    let plain = draft(&t, &[CUPS], &[("delivery_destination", "domestic")]).await;
    submit(&t, &plain).await.assert_status(303);
    let plain = reload(&t, plain.id).await;

    let refused = amend(
        &t,
        &plain,
        "Add lids",
        &[
            ("add_item", LIDS),
            ("add_quantity", "3"),
            ("add_price", "2.00"),
        ],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Lids are export only"),
        "{}",
        refused.text()
    );
    assert_eq!(
        reload(&t, plain.id).await.revision,
        1,
        "nothing was written"
    );

    amend(
        &t,
        &plain,
        "More cups",
        &[
            ("add_item", CUPS),
            ("add_quantity", "3"),
            ("add_price", "2.00"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(reload(&t, plain.id).await.revision, 2);

    // An order abroad holding lids cannot be moved to the UAE.
    let abroad = draft(
        &t,
        &[LIDS],
        &[
            ("delivery_destination", "export"),
            ("delivery_country", "OM"),
        ],
    )
    .await;
    submit(&t, &abroad).await.assert_status(303);
    let abroad = reload(&t, abroad.id).await;
    let moved = amend(
        &t,
        &abroad,
        "Deliver in Dubai instead",
        &[
            ("delivery_destination", "domestic"),
            ("delivery_country", "AE"),
            ("delivery_emirate", "Dubai"),
        ],
    )
    .await;
    moved.assert_status(422);
    assert!(
        moved.text().contains("Lids are export only"),
        "{}",
        moved.text()
    );
    assert_eq!(
        reload(&t, abroad.id).await.delivery_destination.as_deref(),
        Some("export")
    );

    // An order abroad may add lids; a domestic one may move abroad.
    amend(
        &t,
        &abroad,
        "More lids",
        &[
            ("add_item", LIDS),
            ("add_quantity", "3"),
            ("add_price", "2.00"),
        ],
    )
    .await
    .assert_status(303);
    amend(
        &t,
        &plain,
        "Ship it to Muscat",
        &[
            ("delivery_destination", "export"),
            ("delivery_country", "OM"),
        ],
    )
    .await
    .assert_status(422); // revision moved: the form was opened on revision 1
    let plain = reload(&t, plain.id).await;
    amend(
        &t,
        &plain,
        "Ship it to Muscat",
        &[
            ("delivery_destination", "export"),
            ("delivery_country", "OM"),
        ],
    )
    .await
    .assert_status(303);
    let now = reload(&t, plain.id).await;
    assert_eq!(now.delivery_destination.as_deref(), Some("export"));
    assert_eq!(now.delivery_country.as_deref(), Some("OM"));
    amend(
        &t,
        &now,
        "Lids for Muscat",
        &[
            ("add_item", LIDS),
            ("add_quantity", "3"),
            ("add_price", "2.00"),
        ],
    )
    .await
    .assert_status(303);
}

/// The delivery on the order page: the draft's sheet saves it and the page
/// says where the order goes with the VAT category that follows; a delivery
/// that contradicts itself is a 422 in words; a placed order's delivery is
/// changed by amending it.
#[renox::test]
async fn the_order_page_shows_and_saves_where_the_order_goes() {
    let (t, _sam) = boot(true).await;
    let order = draft(&t, &[CUPS], &[]).await;
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Not stated (counts as domestic)")
        .assert_see("Change delivery");

    let contradiction = t
        .app
        .post(
            &format!("/sales-orders/{}/delivery", order.id),
            &[
                ("delivery_destination", "domestic"),
                ("delivery_country", "OM"),
            ],
        )
        .await;
    contradiction.assert_status(422);
    assert!(
        contradiction.text().contains("outside the UAE"),
        "{}",
        contradiction.text()
    );

    t.app
        .post(
            &format!("/sales-orders/{}/delivery", order.id),
            &[
                ("delivery_address", "Plot 12, Jebel Ali Free Zone"),
                ("delivery_emirate", "Dubai"),
                ("delivery_country", "ae"),
            ],
        )
        .await
        .assert_status(303);
    let now = reload(&t, order.id).await;
    assert_eq!(now.delivery_destination.as_deref(), Some("domestic"));
    assert_eq!(now.delivery_country.as_deref(), Some("AE"));
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("In the UAE (domestic)")
        .assert_see("Jebel Ali")
        .assert_see("Standard rated");

    submit(&t, &now).await.assert_status(303);
    let placed = t
        .app
        .post(
            &format!("/sales-orders/{}/delivery", order.id),
            &[("delivery_destination", "export")],
        )
        .await;
    placed.assert_status(422);
    assert!(placed.text().contains("amend it"), "{}", placed.text());
}

/// BL-MFG-11 pt 6: wl-sales provides the kernel's `DeliveryDestination`:
/// export for an order delivered abroad, domestic for one in the UAE, and
/// nothing for an order that does not say or a record that is not an order.
#[renox::test]
async fn wl_sales_tells_the_kernel_where_an_order_is_delivered() {
    let (t, _sam) = boot(true).await;
    let abroad = draft(&t, &[CUPS], &[("delivery_country", "OM")]).await;
    let home = draft(&t, &[CUPS], &[("delivery_destination", "domestic")]).await;
    let unsaid = draft(&t, &[CUPS], &[]).await;
    let kernel = delivery_destination(t.state());
    assert_eq!(
        kernel
            .is_export(t.state(), &abroad.subject())
            .await
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        kernel.is_export(t.state(), &home.subject()).await.unwrap(),
        Some(false)
    );
    assert_eq!(
        kernel
            .is_export(t.state(), &unsaid.subject())
            .await
            .unwrap(),
        None
    );
    let other = Subject::new("inventory.dispatch", home.id);
    assert_eq!(
        kernel.is_export(t.state(), &other).await.unwrap(),
        None,
        "a dispatch is not an order"
    );
}

/// BL-MFG-11 pt 7: a line carries the day it is promised for, from the add
/// line sheet and from the amendment, and the order page shows it; a day that
/// is not a date is a 422 in words.
#[renox::test]
async fn a_line_carries_the_day_it_is_promised_for() {
    let (t, _sam) = boot(true).await;
    let order = draft(&t, &[], &[("delivery_country", "AE")]).await;
    t.app
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", CUPS),
                ("unit_price", "10.00"),
                ("promised", "2027-03-05"),
            ],
        )
        .await
        .assert_status(303);
    let wrong = t
        .app
        .htmx()
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", CUPS),
                ("unit_price", "10.00"),
                ("promised", "next week"),
            ],
        )
        .await;
    wrong.assert_status(422);
    assert!(wrong.text().contains("year-month-day"), "{}", wrong.text());
    let line = lines(&t, &order).await.remove(0);
    assert_eq!(line.promised_on, Some(day("2027-03-05")));
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Promised for")
        .assert_see("05 Mar 2027");

    submit(&t, &order).await.assert_status(303);
    let placed = reload(&t, order.id).await;
    let key = format!("line_{}_promised", line.id);
    amend(
        &t,
        &placed,
        "The customer moved the date",
        &[
            (key.as_str(), "2027-04-10"),
            ("add_item", LIDS),
            ("add_quantity", "1"),
            ("add_price", "1.00"),
            ("add_promised", "2027-04-12"),
        ],
    )
    .await
    .assert_status(422); // lids on a UAE order
    amend(
        &t,
        &placed,
        "The customer moved the date",
        &[
            (key.as_str(), "2027-04-10"),
            ("add_item", CUPS),
            ("add_quantity", "1"),
            ("add_price", "1.00"),
            ("add_promised", "2027-04-12"),
        ],
    )
    .await
    .assert_status(303);
    let all: Vec<SaleLine> = lines(&t, &placed).await;
    assert_eq!(all[0].promised_on, Some(day("2027-04-10")));
    assert_eq!(all[1].promised_on, Some(day("2027-04-12")));
}

/// A domestic draft order with one line of `code` promised for `promised`.
async fn place(t: &Booted, promised: &str, code: &str) -> SalesOrder {
    let order = draft(t, &[], &[("delivery_country", "AE")]).await;
    t.app
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", code),
                ("unit_price", "10.00"),
                ("promised", promised),
            ],
        )
        .await
        .assert_status(303);
    reload(t, order.id).await
}

/// BL-MFG-11 pt 7: wl-sales provides the kernel's `PromisedLines`: the open
/// lines of confirmed orders for an item promised on or after the day asked,
/// oldest first; a draft, a line promised earlier, another item and a
/// cancelled order are not listed.
#[renox::test]
async fn wl_sales_lists_the_confirmed_lines_promised_after_a_day() {
    let (t, _sam) = boot(true).await;
    let item = Subject::new("inventory.item", 42);
    let late = place(&t, "2027-06-20", CUPS).await;
    let soon = place(&t, "2027-06-02", CUPS).await;
    let early = place(&t, "2027-05-01", CUPS).await;
    let other = place(&t, "2027-06-25", "OTHER").await;
    let waiting = place(&t, "2027-07-01", CUPS).await;
    let cancelled = place(&t, "2027-07-02", CUPS).await;
    for order in [&late, &soon, &early, &other, &cancelled] {
        submit(&t, order).await.assert_status(303);
    }
    t.app
        .post(
            &format!("/sales-orders/{}/cancel", cancelled.id),
            &[("reason", "Customer withdrew")],
        )
        .await
        .assert_status(303);

    let found = promised_lines(t.state())
        .of_item(t.state(), &item, day("2027-06-01"))
        .await
        .unwrap();
    let numbers: Vec<String> = found.iter().filter_map(|l| l.number.clone()).collect();
    assert_eq!(
        numbers,
        vec![soon.number.clone(), late.number.clone()],
        "oldest promise first; the draft {}, the early line, the cancelled order and the other item stay out",
        waiting.number
    );
    assert_eq!(found[0].promised, day("2027-06-02"));
    assert_eq!(found[0].order.0, soon.id);
    assert_eq!(found[0].item, item);
}
