//! The brands, outlets and menu items of the kitchens (06 F&B §6 *Key
//! entities*), as far as the orders need them: the brand's delivery promise,
//! the outlet's stock location, and the recipe of a menu item (what one unit
//! consumes), which the stock side of a sale line reads when food is scrapped
//! (`SaleLineStockSource`, `FNB.ACL.16`).
//!
//! Menu management (prices per channel, availability, promotions) is the menu
//! manager's work and comes with its own issues; this is the base they extend.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::business_day::{DEFAULT_CUTOFF_MINUTES, business_date_at, closes_at};

/// The subject type of a brand.
pub const BRAND_SUBJECT_TYPE: &str = "fnb.brand";

/// The subject type of an outlet; also the scope type roles are assigned by.
pub const OUTLET_SUBJECT_TYPE: &str = "fnb.outlet";

/// The delivery promise of a brand that sets none, in minutes (§4.2).
pub const DEFAULT_PROMISE_MINUTES: i64 = 45;

/// A brand: one virtual restaurant on the menus of the channels.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_brands", unique(code))]
pub struct Brand {
    /// The primary key.
    pub id: i64,
    /// A short key, e.g. `zaatar`.
    pub code: String,
    /// The name shown on the screens.
    pub name: String,
    /// The delivery promise in minutes: the perishable hand-over window of
    /// its deliveries (BL-DLV-10).
    pub promise_minutes: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Brand {
    /// The brand `code`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<Brand>> {
        Brand::query().where_(Brand::CODE.eq(code)).first(db).await
    }
}

/// An outlet: a kitchen, a counter or the central kitchen. The scope unit of
/// the roles (§3.2).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_outlets", unique(code))]
pub struct Outlet {
    /// The primary key.
    pub id: i64,
    /// A short key, e.g. `marina`.
    pub code: String,
    /// The name shown on the screens.
    pub name: String,
    /// The `wl-inventory` location the kitchen takes its ingredients from.
    pub location_id: Option<i64>,
    /// The legal entity whose outlet it is.
    pub legal_entity_id: Option<i64>,
    /// Whether the outlet takes orders.
    pub active: bool,
    /// The business-day cut-off in minutes after midnight, Asia/Dubai (04:00
    /// when none is set): orders before it belong to the previous day.
    pub cutoff_minutes: Option<i64>,
    /// A cut-off that applies from [`Outlet::pending_cutoff_from`] only.
    pub pending_cutoff_minutes: Option<i64>,
    /// The first business date the pending cut-off applies to.
    pub pending_cutoff_from: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Outlet {
    /// The cut-off in force for the business date `date`, in minutes after
    /// midnight: a change applies from the next business day only.
    pub fn cutoff_for(&self, date: NaiveDate) -> i64 {
        match (self.pending_cutoff_minutes, self.pending_cutoff_from) {
            (Some(minutes), Some(from)) if date >= from => minutes,
            _ => self.cutoff_minutes.unwrap_or(DEFAULT_CUTOFF_MINUTES),
        }
    }

    /// The business date of an order created at `at`: the date after the
    /// outlet's cut-off (an order at 01:30 belongs to the day before).
    pub fn business_date(&self, at: DateTime) -> NaiveDate {
        let current = business_date_at(at, self.cutoff_minutes.unwrap_or(DEFAULT_CUTOFF_MINUTES));
        if let (Some(minutes), Some(from)) = (self.pending_cutoff_minutes, self.pending_cutoff_from)
        {
            let next = business_date_at(at, minutes);
            if next >= from {
                return next;
            }
        }
        current
    }

    /// The instant the closing of `date` may be submitted: the cut-off on the
    /// next morning.
    pub fn opens_closing_at(&self, date: NaiveDate) -> DateTime {
        closes_at(date, self.cutoff_for(date))
    }

    /// The till (register) of the outlet's daily closing.
    pub fn till_key(&self) -> String {
        format!("{}-till", self.code)
    }

    /// The outlet `code`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<Outlet>> {
        Outlet::query()
            .where_(Outlet::CODE.eq(code))
            .first(db)
            .await
    }

    /// The outlet as a [`Subject`]: `fnb.outlet:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(OUTLET_SUBJECT_TYPE, self.id)
    }

    /// The scope the roles of this outlet are assigned by.
    pub fn scope(&self) -> renox::auth::permissions::Scope {
        renox::auth::permissions::Scope::new(OUTLET_SUBJECT_TYPE, self.id)
    }
}

/// A menu item: what the sale lines sell, with its recipe.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_menu_items", unique(code))]
pub struct MenuItem {
    /// The primary key.
    pub id: i64,
    /// The item code of the sale lines.
    pub code: String,
    /// The name shown on the screens.
    pub name: String,
    /// The brand it belongs to.
    pub brand_id: Option<i64>,
    /// The price of one unit before VAT, in fils.
    pub price: i64,
    /// What one unit costs the kitchen, in fils: its recipe at cost.
    pub unit_cost: i64,
    /// Whether the outlets can sell it now.
    pub available: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl MenuItem {
    /// The menu item `code`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<MenuItem>> {
        MenuItem::query()
            .where_(MenuItem::CODE.eq(code))
            .first(db)
            .await
    }

    /// The recipe: what one unit consumes.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn recipe(&self, db: &Db) -> Result<Vec<RecipeLine>> {
        RecipeLine::query()
            .where_(RecipeLine::MENU_ITEM_ID.eq(self.id))
            .order_by(RecipeLine::ID)
            .get(db)
            .await
    }
}

/// One ingredient of a recipe.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_recipe_lines", index(menu_item_id))]
pub struct RecipeLine {
    /// The primary key.
    pub id: i64,
    /// The menu item.
    #[model(references = "fnb_menu_items")]
    pub menu_item_id: i64,
    /// The stock item (`wl-inventory`).
    pub item_id: i64,
    /// How much of it one unit consumes, in thousandths of its base unit.
    pub per_unit_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}
