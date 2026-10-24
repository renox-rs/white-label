//! Helpers of the batch, cold unit and recall tests: the central kitchen with
//! its lot-tracked ingredients and sauce, the people who produce, decide a
//! hold and approve a wastage, lots received with their expiry, holds placed
//! the way a recall or a reading places them, and a batch planned through the
//! real route.

#![allow(dead_code)]

use fnb::permissions as perm;
use fnb::{BatchRecipe, BatchRecipeLine, Outlet, ProductionBatch};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_inventory::hold_service::{NewHold, NewScope, place};
use wl_inventory::item::value_of;
use wl_inventory::permissions as iperm;
use wl_inventory::{Item, Location, Lot, LotStatus, MovementType, StockMovement};
use wl_kernel::Subject;
use wl_kernel::documents::today;

use super::{World, make_user};

/// The central kitchen of the tests.
pub struct Kitchen {
    /// The head chef: produces, decides hold lines, opens recalls and tells
    /// the customers.
    pub chef: User,
    /// The kitchen manager: the chef's deputy and the second person of a
    /// wastage above AED 200.
    pub manager: User,
    /// The owner: decides the public notice of a recall.
    pub owner: User,
    /// Tomatoes, lot-tracked.
    pub tomato: Item,
    /// Tomato sauce, lot-tracked: what the batches make.
    pub sauce: Item,
    /// The central kitchen's stock location.
    pub central: Location,
    /// The recipe: 12 kg of tomatoes make 10 kg of sauce.
    pub recipe: BatchRecipe,
    /// Its only ingredient line.
    pub line: BatchRecipeLine,
}

/// The central kitchen made and its outlet pointed at the stock location.
pub async fn kitchen(w: &World) -> Kitchen {
    let chef = make_user(
        w.db(),
        "Chef Samir",
        &[
            perm::PRODUCTION_VIEW,
            perm::PRODUCTION_MANAGE,
            perm::TEMPERATURE_VIEW,
            perm::TEMPERATURE_LOG,
            perm::RECALL_TRACE_VIEW,
            perm::RECALL_TRACE_DECIDE,
            iperm::LOTS_RELEASE,
            iperm::STOCK_HOLD,
            iperm::STOCK_VIEW,
            iperm::RECALLS_MANAGE,
            iperm::RECALLS_VIEW,
        ],
    )
    .await;
    let manager = make_user(
        w.db(),
        "Mounir",
        &[
            perm::PRODUCTION_VIEW,
            perm::TEMPERATURE_VIEW,
            perm::TEMPERATURE_LOG,
            perm::COLD_UNITS_MANAGE,
            perm::RECALL_TRACE_VIEW,
            iperm::LOTS_RELEASE,
            iperm::STOCK_APPROVE,
            iperm::STOCK_VIEW,
        ],
    )
    .await;
    let owner = make_user(
        w.db(),
        "Fadi",
        &[
            perm::PRODUCTION_VIEW,
            perm::RECALL_TRACE_VIEW,
            perm::RECALL_TRACE_DECIDE,
            iperm::RECALLS_MANAGE,
            iperm::RECALLS_VIEW,
        ],
    )
    .await;
    let mut tomato = Item::new("TOMATO", "Tomato", "kg").lot_tracked();
    tomato.average_cost = 800;
    tomato.save(w.db()).await.unwrap();
    let mut sauce = Item::new("TOMATO-SAUCE", "Tomato sauce", "kg").lot_tracked();
    sauce.average_cost = 1_500;
    sauce.save(w.db()).await.unwrap();
    let mut central = Location::new("CENTRAL", "Central Kitchen");
    central.save(w.db()).await.unwrap();
    let mut outlet = Outlet::by_code(w.db(), "central")
        .await
        .unwrap()
        .expect("the demo catalogue has the central kitchen");
    outlet.location_id = Some(central.id);
    outlet.save(w.db()).await.unwrap();
    let recipe = fnb::seed::demo_batch_recipe(
        w.db(),
        "tomato-sauce",
        "Tomato sauce",
        sauce.id,
        10_000,
        &[(tomato.id, "TOMATO", 12_000)],
    )
    .await
    .unwrap();
    let line = recipe.lines(w.db()).await.unwrap().remove(0);
    Kitchen {
        chef,
        manager,
        owner,
        tomato,
        sauce,
        central,
        recipe,
        line,
    }
}

/// Lot `number` of `item`: `quantity_milli` received at `location`, expiring
/// in `days` days from today.
pub async fn lot(
    w: &World,
    item: &Item,
    number: &str,
    location: &Location,
    quantity_milli: i64,
    days: i64,
) -> Lot {
    let source = Subject::new("inventory.goods_receipt", 9_900);
    let mut lot = Lot {
        item_id: item.id,
        number: number.to_owned(),
        source_type: Some(source.subject_type.clone()),
        source_id: Some(source.subject_id),
        expiry_date: Some(today() + TimeDelta::days(days)),
        status: LotStatus::Released,
        ..Lot::default()
    };
    lot.save(w.db()).await.unwrap();
    let mut moved = StockMovement::receive(
        item.id,
        Some(lot.id),
        location.id,
        quantity_milli,
        item.average_cost,
        value_of(quantity_milli, item.average_cost),
        &source,
        None,
    );
    moved.save(w.db()).await.unwrap();
    lot
}

/// Holds the whole `lot` with `reason`, the way a recall does (the system
/// places it; `LotHeld` follows).
pub async fn hold(w: &World, lot: &Lot, reason: &str) -> wl_inventory::StockHold {
    place(
        w.state(),
        NewHold {
            reason: reason.to_owned(),
            reference: Subject::new("test.cause", 1),
            scope: NewScope::Lots(vec![lot.id]),
            by: None,
        },
    )
    .await
    .expect("the hold is placed")
}

/// A batch of the recipe planned by `chef` through `POST /batches`, at the
/// central kitchen, for 10 kg.
pub async fn plan(w: &World, k: &Kitchen) -> ProductionBatch {
    let recipe = k.recipe.id.to_string();
    let outlet = Outlet::by_code(w.db(), "central")
        .await
        .unwrap()
        .expect("the central kitchen")
        .id
        .to_string();
    w.post(
        &k.chef,
        "/batches",
        &[
            ("recipe", recipe.as_str()),
            ("yield", "10"),
            ("outlet", outlet.as_str()),
        ],
    )
    .await
    .assert_status(303);
    ProductionBatch::query()
        .order_by_desc(ProductionBatch::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("the batch is planned")
}

/// `POST /batches/{id}/start` as `by` picking `lot` for the only ingredient.
pub async fn start(
    w: &World,
    by: &User,
    k: &Kitchen,
    batch: &ProductionBatch,
    lot: &Lot,
) -> TestResponse {
    let name = format!("lot_{}", k.line.id);
    let id = lot.id.to_string();
    w.post(
        by,
        &format!("/batches/{}/start", batch.id),
        &[(name.as_str(), id.as_str())],
    )
    .await
}

/// `POST /batches/{id}/finish` as `by`: `yielded` kg, expiring `days` days
/// from today.
pub async fn finish(
    w: &World,
    by: &User,
    batch: &ProductionBatch,
    yielded: &str,
    days: i64,
) -> TestResponse {
    let expiry = (today() + TimeDelta::days(days)).to_string();
    w.post(
        by,
        &format!("/batches/{}/finish", batch.id),
        &[("actual_yield", yielded), ("expiry", expiry.as_str())],
    )
    .await
}

/// `batch`, as stored now.
pub async fn reload(w: &World, batch: &ProductionBatch) -> ProductionBatch {
    ProductionBatch::find_or_404(w.db(), batch.id)
        .await
        .unwrap()
}

/// `lot`, as stored now.
pub async fn lot_now(w: &World, lot: &Lot) -> Lot {
    Lot::find_or_404(w.db(), lot.id).await.unwrap()
}

/// The lot a finished batch made.
pub async fn made_lot(w: &World, batch: &ProductionBatch) -> Lot {
    let id = reload(w, batch).await.lot_id.expect("the batch made a lot");
    Lot::find_or_404(w.db(), id).await.unwrap()
}

/// The wastage movements of `lot`.
pub async fn wastage_of(w: &World, lot: &Lot) -> Vec<StockMovement> {
    StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(lot.id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Wastage))
        .get(w.db())
        .await
        .unwrap()
}
