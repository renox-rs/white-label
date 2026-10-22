//! The F&B demo catalogue (DEMO-DATA.md §06, *Zaatar & Co. Kitchens*): the
//! brands, the outlets, the menu items and their recipes. Idempotent: a row
//! whose code exists is kept as it is. The host's demo adds the people, the
//! customers and the orders (`host/src/app/fnb_demo.rs`), because an order is
//! a sales order and the app does not depend on `wl-sales`.

use renox::prelude::*;

use crate::catalogue::{Brand, MenuItem, Outlet, RecipeLine};

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
