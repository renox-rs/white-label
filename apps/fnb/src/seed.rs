//! The F&B demo catalogue (DEMO-DATA.md §06, *Zaatar & Co. Kitchens*): the
//! brands, the outlets, the menu items and their recipes. Idempotent: a row
//! whose code exists is kept as it is. The host's demo adds the people, the
//! customers and the orders (`host/src/app/fnb_demo.rs`), because an order is
//! a sales order and the app does not depend on `wl-sales`.

use renox::prelude::*;

use crate::catalogue::{Brand, MenuItem, Outlet, RecipeLine};
use crate::payout::{ImportPreset, ReasonMap};

/// The brands: code, name, delivery promise in minutes.
pub const BRANDS: [(&str, &str, i64); 3] = [
    ("zaatar", "Zaatar Grill", 45),
    ("habibi", "Habibi Bowls", 40),
    ("sweet", "Sweet Tray", 50),
];

/// The outlets: code, name.
pub const OUTLETS: [(&str, &str); 3] = [
    ("marina", "Marina Kitchen"),
    ("sharjah", "Sharjah Kitchen"),
    ("central", "Central Production Kitchen"),
];

/// The menu items: code, name, brand code, price (fils, before VAT), unit
/// cost (fils), and the recipe as (ingredient key, thousandths per unit).
pub const MENU: [(&str, &str, &str, i64, i64, &[(&str, i64)]); 4] = [
    (
        "shawarma-plate",
        "Chicken shawarma plate",
        "zaatar",
        3_000,
        1_100,
        &[("chicken", 250), ("rice", 150)],
    ),
    (
        "falafel-wrap",
        "Falafel wrap",
        "habibi",
        1_800,
        600,
        &[("bread", 1_000), ("hummus", 80)],
    ),
    (
        "hummus-bowl",
        "Hummus bowl",
        "habibi",
        1_500,
        450,
        &[("hummus", 200), ("bread", 1_000)],
    ),
    (
        "kunafa-tray",
        "Kunafa tray",
        "sweet",
        3_500,
        1_300,
        &[("bread", 500)],
    ),
];

/// What the demo catalogue made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Installed {
    /// Brands added.
    pub brands: usize,
    /// Outlets added.
    pub outlets: usize,
    /// Menu items added.
    pub items: usize,
}

/// Adds the demo brands, outlets (the first one with the stock location
/// `location_id`) and menu items; `ingredients` maps the recipe keys
/// (`chicken`, `rice`, `bread`, `hummus`) to stock items. A recipe line whose
/// ingredient is not given is left out.
///
/// # Errors
///
/// Database errors.
pub async fn demo_catalogue(
    db: &Db,
    location_id: Option<i64>,
    ingredients: &[(&str, i64)],
) -> Result<Installed> {
    let mut installed = Installed::default();
    for (code, name, promise) in BRANDS {
        if Brand::by_code(db, code).await?.is_none() {
            let mut brand = Brand {
                code: code.to_owned(),
                name: name.to_owned(),
                promise_minutes: promise,
                ..Brand::default()
            };
            brand.insert(db).await?;
            installed.brands += 1;
        }
    }
    for (n, (code, name)) in OUTLETS.iter().enumerate() {
        if Outlet::by_code(db, code).await?.is_none() {
            let mut outlet = Outlet {
                code: (*code).to_owned(),
                name: (*name).to_owned(),
                location_id: if n == 0 { location_id } else { None },
                active: true,
                ..Outlet::default()
            };
            outlet.insert(db).await?;
            installed.outlets += 1;
        }
    }
    for (code, name, brand, price, cost, recipe) in MENU {
        if MenuItem::by_code(db, code).await?.is_some() {
            continue;
        }
        let brand_id = Brand::by_code(db, brand).await?.map(|brand| brand.id);
        let mut item = MenuItem {
            code: code.to_owned(),
            name: name.to_owned(),
            brand_id,
            price,
            unit_cost: cost,
            available: true,
            ..MenuItem::default()
        };
        item.insert(db).await?;
        for (key, per_unit) in recipe {
            if let Some((_, stock)) = ingredients.iter().find(|(k, _)| k == key) {
                let mut line = RecipeLine {
                    menu_item_id: item.id,
                    item_id: *stock,
                    per_unit_milli: *per_unit,
                    ..RecipeLine::default()
                };
                line.insert(db).await?;
            }
        }
        installed.items += 1;
    }
    Ok(installed)
}

/// The column presets of the aggregators' payout files: aggregator, name,
/// delimiter and the headers of the order number, gross, commission,
/// promotions, refund, net and refund reason. The real files are not
/// published; these are the demo's, built the way onboarding builds the first
/// client's (`FNB.REC.01`, decided 2026-10-15).
pub const PAYOUT_PRESETS: [(&str, &str, &str, [&str; 7]); 3] = [
    (
        "aggregator_a",
        "Aggregator A weekly CSV",
        ",",
        [
            "Order ID",
            "Gross",
            "Commission",
            "Promo",
            "Refund",
            "Net",
            "Refund reason",
        ],
    ),
    (
        "aggregator_b",
        "Aggregator B settlement CSV",
        ";",
        [
            "Order no",
            "Order value",
            "Fee",
            "Discount",
            "Refunded",
            "Payout",
            "Reason",
        ],
    ),
    (
        "aggregator_c",
        "Aggregator C statement CSV",
        ",",
        [
            "Reference",
            "Total",
            "Commission",
            "Promotion",
            "Refund",
            "Net",
            "Refund note",
        ],
    ),
];

/// The aggregators' refund reasons the demo maps: aggregator, the reason as
/// printed, the complaint category and the proposed responsible party. A
/// reason not here (*Customer changed mind*) is set by the accountant.
pub const PAYOUT_REASONS: [(&str, &str, &str, &str); 5] = [
    ("aggregator_a", "Food was cold", "cold", "business"),
    ("aggregator_a", "Missing item", "missing_item", "business"),
    ("aggregator_a", "Wrong item", "wrong_item", "business"),
    ("aggregator_a", "Rider was late", "late", "aggregator"),
    ("aggregator_b", "Cold food", "cold", "business"),
];

/// Adds the column presets and the refund reason mappings the database lacks;
/// what exists is kept as the client changed it. Returns how many rows it
/// added.
///
/// # Errors
///
/// Database errors.
pub async fn install_payouts(db: &Db) -> Result<usize> {
    let mut added = 0;
    for (aggregator, name, delimiter, columns) in PAYOUT_PRESETS {
        if ImportPreset::of(db, aggregator).await?.is_some() {
            continue;
        }
        let [order, gross, commission, promo, refund, net, reason] = columns;
        let mut preset = ImportPreset {
            aggregator: aggregator.to_owned(),
            name: name.to_owned(),
            delimiter: delimiter.to_owned(),
            order_column: order.to_owned(),
            gross_column: gross.to_owned(),
            commission_column: commission.to_owned(),
            promo_column: promo.to_owned(),
            refund_column: refund.to_owned(),
            net_column: net.to_owned(),
            reason_column: reason.to_owned(),
            ..ImportPreset::default()
        };
        preset.insert(db).await?;
        added += 1;
    }
    for (aggregator, reason, category, party) in PAYOUT_REASONS {
        if ReasonMap::find_for(db, aggregator, reason).await?.is_some() {
            continue;
        }
        let mut map = ReasonMap {
            aggregator: aggregator.to_owned(),
            reason_key: reason.trim().to_lowercase(),
            category: category.to_owned(),
            party: party.to_owned(),
            ..ReasonMap::default()
        };
        map.insert(db).await?;
        added += 1;
    }
    Ok(added)
}
