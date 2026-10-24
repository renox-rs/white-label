//! The receive-stock screens: the purchase orders waiting for goods, the goods
//! receipts and one receipt with its lines (BL-STK-01, §5.1).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /purchase-orders` | `wl_inventory.orders` | `purchase_orders.view`, `purchase_orders.issue` or `goods_receipts.post` somewhere |
//! | `POST /purchase-orders/{id}/receive` | `wl_inventory.order_receive` | `goods_receipts.post` at the order's location |
//! | `GET /goods-receipts` | `wl_inventory.receipts` | `goods_receipts.post` or `.reverse` somewhere, or `stock.view` |
//! | `GET /goods-receipts/{id}` | `wl_inventory.receipt` | the same |
//!
//! *Start a receipt* makes a draft receipt of the order's outstanding lines,
//! each received and accepted in full; the storekeeper then enters what really
//! arrived on each line (the received, accepted and rejected quantity, the
//! reason and what became of the rejected goods, the lot and its expiry) and
//! posts. Saving a line and posting use the routes of [`crate::pages`]; a post
//! above the over-receipt tolerance waits for a second person and shows its
//! approval card.

use std::collections::BTreeMap;

use renox::chrono::NaiveDate;
use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::approvals::{self, ApprovalCard};

use crate::goods_receipt::{
    DraftLine, DraftReceipt, GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus, create_draft,
    lines_of as receipt_lines,
};
use crate::item::Item;
use crate::location::Location;
use crate::permissions;
use crate::purchase_order::{PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, lines_of};
use crate::screens::quantity_text;
use crate::stock_screens::{Texts, amount, item_name, location_name, tr, tx, word};

/// The routes of the receive-stock screens.
pub fn routes() -> Routes {
    Routes::new()
        .get("/purchase-orders", orders)
        .name("wl_inventory.orders")
        .post("/purchase-orders/{id}/receive", start_receipt)
        .name("wl_inventory.order_receive")
        .get("/goods-receipts", receipts)
        .name("wl_inventory.receipts")
        .get("/goods-receipts/{id}", receipt)
        .name("wl_inventory.receipt")
        .require_auth()
}

fn may_see(user: &AuthUser) -> bool {
    [
        permissions::PURCHASE_ORDERS_VIEW,
        permissions::PURCHASE_ORDERS_ISSUE,
        permissions::GOODS_RECEIPTS_POST,
        permissions::GOODS_RECEIPTS_REVERSE,
        permissions::STOCK_VIEW,
    ]
    .iter()
    .any(|permission| user.has_permission_anywhere(permission))
}

async fn supplier_name(state: &AppState, lang: &Lang, id: i64) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// What the orders page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/orders.html")]
pub(crate) struct OrdersPage {
    tx: Texts,
    orders: GridPage<PurchaseOrder>,
}

/// The purchase orders grid: the orders open for receiving, with the store,
/// the day they are expected, what is still to come and the buttons that
/// start (or open) a receipt.
fn orders_grid(lang: &Lang) -> Grid {
    let t = tx(lang);
    Grid::new("wl-inventory-orders")
        .title(&t["orders.title"])
        .column(
            Column::text("number", &t["orders.number"])
                .searchable()
                .frozen()
                .mobile(),
        )
        .column(
            Column::text("supplier", &t["orders.supplier"])
                .sortable(false)
                .filterable(false)
                .mobile(),
        )
        .column(
            Column::related(
                "store",
                &t["stock.store"],
                "wl_inventory_locations",
                "location_id",
                "name_en",
            )
            .searchable(),
        )
        .column(Column::date("expected_date", &t["orders.expected"]))
        .column(
            Column::text("outstanding", &t["orders.outstanding"])
                .sortable(false)
                .filterable(false)
                .wrap()
                .mobile(),
        )
        .column(
            Column::select(
                "status",
                &t["fields.status"],
                ["issued", "partially_received"]
                    .map(|status| (status, word(lang, "order", status))),
            )
            .badges(&[("issued", "info"), ("partially_received", "warning")])
            .mobile(),
        )
        .column(Column::custom("actions", "").frozen_right().mobile())
        .sort_by("-id")
        .cards_on_mobile()
        .empty_state(&t["orders.empty"], Some(&t["orders.empty_hint"]))
}

/// What the grid adds to a purchase order: what is still to come, and the
/// buttons.
struct OrderExtra {
    supplier: String,
    outstanding: String,
    can_receive: bool,
    draft: String,
}

/// `GET /purchase-orders`: the orders open for receiving, with what is still
/// to come on each.
async fn orders(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    if !may_see(&user) {
        return Err(Error::Forbidden);
    }
    let query = PurchaseOrder::query().where_(PurchaseOrder::STATUS.is_in([
        PurchaseOrderStatus::Issued,
        PurchaseOrderStatus::PartiallyReceived,
    ]));
    let page = orders_grid(&lang).page(query, &request).await?;
    let mut extras = BTreeMap::new();
    for order in page.items() {
        let location = Location::find_or_404(&state.db, order.location_id).await?;
        let mut outstanding = Vec::new();
        for line in lines_of(&state.db, order.id).await? {
            let left = line.outstanding_milli();
            if left > 0 {
                let item = Item::find_or_404(&state.db, line.item_id).await?;
                outstanding.push(format!(
                    "{} · {}",
                    item_name(&item, &lang),
                    amount(left, &item.base_unit)
                ));
            }
        }
        // A receipt already started for the order is opened, not started again.
        let draft = GoodsReceipt::query()
            .where_(GoodsReceipt::PURCHASE_ORDER_ID.eq(Some(order.id)))
            .where_(GoodsReceipt::STATUS.eq(GoodsReceiptStatus::Draft))
            .first(&state.db)
            .await?
            .map(|receipt| format!("/goods-receipts/{}", receipt.id))
            .unwrap_or_default();
        extras.insert(
            order.id,
            OrderExtra {
                supplier: supplier_name(&state, &lang, order.supplier_party_id).await,
                outstanding: outstanding.join(", "),
                can_receive: draft.is_empty()
                    && user.has_permission_in(permissions::GOODS_RECEIPTS_POST, &location.scope()),
                draft,
            },
        );
    }
    let orders = page.extend(|order| match extras.get(&order.id) {
        Some(extra) => renox::serde_json::json!({
            "supplier": extra.supplier,
            "outstanding": extra.outstanding,
            "can_receive": extra.can_receive,
            "draft": extra.draft,
        }),
        None => renox::serde_json::json!({}),
    });
    Ok(View::typed(OrdersPage {
        tx: tx(&lang),
        orders,
    }))
}

/// What the storekeeper enters to start a receipt.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct StartForm {
    /// The supplier's delivery note number.
    #[validate(max = 64)]
    pub delivery_note: Option<String>,
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `POST /purchase-orders/{id}/receive`: a draft receipt of the outstanding
/// lines, each received and accepted in full, at the order's location; the
/// answer goes to the receipt's page.
async fn start_receipt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<StartForm>,
) -> Result<Response> {
    let order = PurchaseOrder::find_or_404(&state.db, id).await?;
    let location = Location::find_or_404(&state.db, order.location_id).await?;
    if !user.has_permission_in(permissions::GOODS_RECEIPTS_POST, &location.scope()) {
        return Err(Error::Forbidden);
    }
    if !order.is_open_for_receiving() {
        return Err(refused(tr(&lang, "problem.order_closed")));
    }
    let lines: Vec<DraftLine> = lines_of(&state.db, order.id)
        .await?
        .iter()
        .filter(|line: &&PurchaseOrderLine| line.outstanding_milli() > 0)
        .map(|line| DraftLine::on(line.id, line.outstanding_milli(), line.outstanding_milli()))
        .collect();
    if lines.is_empty() {
        return Err(refused(tr(&lang, "problem.nothing_outstanding")));
    }
    let receipt = create_draft(
        &state.db,
        Some(user.id),
        DraftReceipt {
            purchase_order_id: order.id,
            location_id: location.id,
            delivery_note: form.delivery_note.clone(),
            lines,
        },
    )
    .await?;
    Ok((
        Toast::success(tr(&lang, "receipt_started")),
        renox::HxRedirect(format!("/goods-receipts/{}", receipt.id)),
    )
        .into_response())
}

/// What the receipts page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/receipts.html")]
pub(crate) struct ReceiptsPage {
    tx: Texts,
    receipts: GridPage<GoodsReceipt>,
}

fn receipt_status(status: GoodsReceiptStatus) -> &'static str {
    match status {
        GoodsReceiptStatus::Draft => "warning",
        GoodsReceiptStatus::Posted => "success",
        GoodsReceiptStatus::Reversed => "error",
    }
}

/// The title of a receipt: its number, or "Draft receipt" until it is posted.
fn receipt_title(lang: &Lang, receipt: &GoodsReceipt) -> String {
    match &receipt.number {
        Some(number) => number.clone(),
        None => tr(lang, "receipt_draft"),
    }
}

/// The goods receipts grid, drafts first: receipt, purchase order, supplier,
/// store, delivery note, status and when it was posted (or started).
fn receipts_grid(lang: &Lang) -> Grid {
    let t = tx(lang);
    Grid::new("wl-inventory-receipts")
        .title(&t["receipts.title"])
        .column(
            Column::text("number", &t["receipts.receipt"])
                .link("/goods-receipts/{id}")
                .searchable()
                .frozen()
                .mobile(),
        )
        .column(
            Column::related(
                "order_number",
                &t["orders.number"],
                "wl_inventory_purchase_orders",
                "purchase_order_id",
                "number",
            )
            .searchable(),
        )
        .column(
            Column::text("supplier", &t["orders.supplier"])
                .sortable(false)
                .filterable(false)
                .mobile(),
        )
        .column(
            Column::related(
                "store",
                &t["stock.store"],
                "wl_inventory_locations",
                "location_id",
                "name_en",
            )
            .searchable(),
        )
        .column(Column::text("delivery_note", &t["receipts.delivery_note"]).searchable())
        .column(
            Column::select(
                "status",
                &t["fields.status"],
                ["draft", "posted", "reversed"]
                    .map(|status| (status, word(lang, "receipt", status))),
            )
            .badges(&[
                ("draft", "warning"),
                ("posted", "success"),
                ("reversed", "danger"),
            ])
            .mobile(),
        )
        .column(
            Column::datetime("happened_at", &t["transfers.when"])
                .computed("COALESCE({T}.\"posted_at\", {T}.\"created_at\")"),
        )
        .sort_by("status,-id")
        .row_url("/goods-receipts/{id}")
        .cards_on_mobile()
        .empty_state(&t["receipts.empty"], Some(&t["receipts.empty_hint"]))
}

/// `GET /goods-receipts`: the receipts, drafts first, at the stores the user
/// may receive at.
async fn receipts(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<View> {
    if !may_see(&user) {
        return Err(Error::Forbidden);
    }
    let query = crate::grids::within_reach(
        &state.db,
        &user,
        &[permissions::GOODS_RECEIPTS_POST, permissions::STOCK_VIEW],
        GoodsReceipt::query(),
        "scope_type",
        "scope_id",
    )
    .await?;
    let page = receipts_grid(&lang).page(query, &request).await?;
    let mut suppliers = BTreeMap::new();
    for receipt in page.items() {
        suppliers.insert(
            receipt.id,
            supplier_name(&state, &lang, receipt.supplier_party_id).await,
        );
    }
    let receipts = page.extend(|receipt| {
        renox::serde_json::json!({
            "number": receipt_title(&lang, receipt),
            "supplier": suppliers.get(&receipt.id),
        })
    });
    Ok(View::typed(ReceiptsPage {
        tx: tx(&lang),
        receipts,
    }))
}

/// One line of a receipt.
#[derive(Serialize, Debug, Clone)]
pub struct LineView {
    id: i64,
    position: i64,
    item: String,
    unit: String,
    ordered: String,
    received: String,
    accepted: String,
    rejected: String,
    reason: String,
    reason_code: String,
    handling_code: String,
    outcome_code: String,
    lot_tracked: bool,
    lot: String,
    expiry: String,
    evidence: String,
}

/// What the receipt page needs.
#[derive(Serialize, ViewData)]
#[view(template = "wl-inventory/receipt.html")]
pub(crate) struct ReceiptPage {
    tx: Texts,
    id: i64,
    title: String,
    status: String,
    status_kind: &'static str,
    order: String,
    supplier: String,
    location: String,
    delivery_note: String,
    lines: Vec<LineView>,
    /// The warning that a unit the lines go to is on hold (empty when none).
    warning: String,
    can_edit: bool,
    can_post: bool,
    can_reverse: bool,
    reasons: Vec<(String, String)>,
    handlings: Vec<(String, String)>,
    outcomes: Vec<(String, String)>,
    approvals: Vec<ApprovalCard>,
}

/// `GET /goods-receipts/{id}`.
async fn receipt(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let receipt = GoodsReceipt::find_or_404(&state.db, id).await?;
    let poster = user.has_permission_in(permissions::GOODS_RECEIPTS_POST, &receipt.scope());
    let reverser = user.has_permission_in(permissions::GOODS_RECEIPTS_REVERSE, &receipt.scope());
    if !poster && !reverser && !user.has_permission_in(permissions::STOCK_VIEW, &receipt.scope()) {
        return Err(Error::Forbidden);
    }
    let location = Location::find_or_404(&state.db, receipt.location_id).await?;
    let order = match receipt.purchase_order_id {
        Some(order_id) => PurchaseOrder::find(&state.db, order_id).await?,
        None => None,
    };
    let order_lines: Vec<PurchaseOrderLine> = match &order {
        Some(order) => lines_of(&state.db, order.id).await?,
        None => Vec::new(),
    };
    let mut lines = Vec::new();
    let mut item_ids = Vec::new();
    for line in receipt_lines(&state.db, receipt.id).await? {
        item_ids.push(line.item_id);
        lines.push(line_view(&state, &lang, &line, &order_lines).await?);
    }
    // A receipt into a unit on temperature hold says so, as a transfer does.
    let warning = if receipt.status == GoodsReceiptStatus::Draft {
        crate::hold_service::warnings_for(&state.db, receipt.location_id, &item_ids)
            .await?
            .join(" ")
    } else {
        String::new()
    };
    let pending = approvals::pending_for(&state.db, &receipt.subject()).await?;
    let mut cards = Vec::new();
    for request in &pending {
        cards.push(approvals::card(&state.db, request, &user, &lang).await?);
    }
    let draft = receipt.status == GoodsReceiptStatus::Draft;
    let reasons = std::iter::once((String::new(), tr(&lang, "no_reason")))
        .chain(
            crate::config_of(&state)
                .rejection_reasons
                .iter()
                .map(|reason| (reason.code.clone(), word(&lang, "reject", &reason.code))),
        )
        .collect();
    Ok(View::typed(ReceiptPage {
        tx: tx(&lang),
        id: receipt.id,
        title: receipt_title(&lang, &receipt),
        status: word(&lang, "receipt", receipt.status.as_str()),
        status_kind: receipt_status(receipt.status),
        order: order.and_then(|order| order.number).unwrap_or_default(),
        supplier: supplier_name(&state, &lang, receipt.supplier_party_id).await,
        location: location_name(&location),
        delivery_note: receipt.delivery_note.clone().unwrap_or_default(),
        lines,
        warning,
        can_edit: draft && poster && pending.is_empty(),
        can_post: draft && poster && pending.is_empty(),
        can_reverse: receipt.status == GoodsReceiptStatus::Posted && reverser,
        reasons,
        handlings: vec![
            (
                "refused_at_door".to_owned(),
                word(&lang, "handling", "refused_at_door"),
            ),
            (
                "kept_for_collection".to_owned(),
                word(&lang, "handling", "kept_for_collection"),
            ),
        ],
        outcomes: vec![
            ("replace".to_owned(), word(&lang, "outcome", "replace")),
            ("credit".to_owned(), word(&lang, "outcome", "credit")),
        ],
        approvals: cards,
    }))
}

async fn line_view(
    state: &AppState,
    lang: &Lang,
    line: &GoodsReceiptLine,
    order_lines: &[PurchaseOrderLine],
) -> Result<LineView> {
    let item = Item::find_or_404(&state.db, line.item_id).await?;
    let ordered = line
        .po_line_id
        .and_then(|id| order_lines.iter().find(|po_line| po_line.id == id))
        .map(|po_line| amount(po_line.quantity_milli, &item.base_unit))
        .unwrap_or_default();
    let reason_code = line.rejection_reason.clone().unwrap_or_default();
    Ok(LineView {
        id: line.id,
        position: line.position,
        item: item_name(&item, lang),
        unit: item.base_unit.clone(),
        ordered,
        received: quantity_text(line.received_qty_milli),
        accepted: quantity_text(line.accepted_qty_milli),
        rejected: quantity_text(line.rejected_qty_milli),
        reason: if reason_code.is_empty() {
            String::new()
        } else {
            word(lang, "reject", &reason_code)
        },
        reason_code,
        handling_code: line
            .rejected_handling
            .map(|handling| handling.as_str().to_owned())
            .unwrap_or_else(|| "refused_at_door".to_owned()),
        outcome_code: line.wanted_outcome.as_str().to_owned(),
        lot_tracked: item.lot_tracked,
        lot: line.lot_number.clone().unwrap_or_default(),
        expiry: line
            .expiry_date
            .map(|day: NaiveDate| day.to_string())
            .unwrap_or_default(),
        evidence: line.evidence.clone().unwrap_or_default(),
    })
}
