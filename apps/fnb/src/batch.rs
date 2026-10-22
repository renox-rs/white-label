//! The production batch and its records (06 F&B §6 *Key entities*, §7.1
//! *Production batch*; the base for BL-FNB-11 and BL-FNB-13).
//!
//! A central kitchen makes a sauce, a marinated protein or a dough in bulk
//! from a [`BatchRecipe`]. The chef plans a [`ProductionBatch`], starts it
//! picking the ingredient lots (recorded as the batch's [`BatchInput`] rows,
//! the lot genealogy a recall follows) and finishes it with the yield and the
//! expiry; the stock module makes the output lot. The status moves only
//! through [`crate::batch_machine`].
//!
//! Recipes are as far as batches need: ingredients and a standard yield. A
//! sub-recipe is a batch recipe whose output is an ingredient of another one.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE};
use wl_kernel::numbering::{Reset, Series};

/// The subject type of a production batch.
pub const SUBJECT_TYPE: &str = "fnb.batch";

/// `BAT-0001`: the number of a batch, also the number of the lot it makes.
pub const BATCH_SERIES: Series = Series {
    key: "fnb_batch",
    pattern: "{prefix}-{0000}",
    prefix: "BAT",
    reset: Reset::Never,
    gapless: false,
    per_entity: false,
};

/// Where a production batch is (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BatchStatus {
    /// Scheduled for production.
    #[default]
    Planned,
    /// Being prepared; the ingredient lots are recorded.
    InProduction,
    /// Made, labelled and in stock with an expiry.
    Produced,
    /// Its lot is held (a recall of an ingredient lot, quality, temperature):
    /// not usable.
    OnHold,
    /// Fully used or transferred (terminal).
    Consumed,
    /// Expired, failed quality or discarded after a hold; logged as wastage
    /// (terminal).
    Discarded,
}

impl BatchStatus {
    /// The words of the status in a key: `in_production`.
    pub fn key(self) -> &'static str {
        match self {
            BatchStatus::Planned => "planned",
            BatchStatus::InProduction => "in_production",
            BatchStatus::Produced => "produced",
            BatchStatus::OnHold => "on_hold",
            BatchStatus::Consumed => "consumed",
            BatchStatus::Discarded => "discarded",
        }
    }
}

/// What a batch is made of.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_batch_recipes", unique(code))]
pub struct BatchRecipe {
    /// The primary key.
    pub id: i64,
    /// A short key, e.g. `garlic-sauce`.
    pub code: String,
    /// The name shown on the screens.
    pub name: String,
    /// The stock item the batch makes (lot-tracked).
    pub output_item_id: i64,
    /// What one standard batch yields, in thousandths of the item's unit.
    pub yield_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BatchRecipe {
    /// The recipe `code`, if there is one.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<BatchRecipe>> {
        BatchRecipe::query()
            .where_(BatchRecipe::CODE.eq(code))
            .first(db)
            .await
    }

    /// The ingredients, in order.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn lines(&self, db: &Db) -> Result<Vec<BatchRecipeLine>> {
        BatchRecipeLine::query()
            .where_(BatchRecipeLine::RECIPE_ID.eq(self.id))
            .order_by(BatchRecipeLine::ID)
            .get(db)
            .await
    }
}

/// One ingredient of a batch recipe.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_batch_recipe_lines", index(recipe_id))]
pub struct BatchRecipeLine {
    /// The primary key.
    pub id: i64,
    /// The recipe.
    #[model(references = "fnb_batch_recipes")]
    pub recipe_id: i64,
    /// The stock item (`wl-inventory`), lot-tracked.
    pub item_id: i64,
    /// Its code: the stock module proposes lots by code.
    pub item_code: String,
    /// What one standard batch uses, in thousandths.
    pub quantity_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BatchRecipeLine {
    /// What a batch with the planned yield `planned_milli` uses of the
    /// ingredient, in thousandths: the standard quantity scaled by the yield.
    pub fn needed(&self, recipe: &BatchRecipe, planned_milli: i64) -> i64 {
        if recipe.yield_milli <= 0 {
            return self.quantity_milli;
        }
        i64::try_from(
            i128::from(self.quantity_milli) * i128::from(planned_milli)
                / i128::from(recipe.yield_milli),
        )
        .unwrap_or(self.quantity_milli)
    }
}

/// A production batch.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "fnb_batches",
    state_machine,
    unique(number),
    index(status),
    index(lot_id)
)]
pub struct ProductionBatch {
    /// The primary key.
    pub id: i64,
    /// The number, e.g. `BAT-0001`; the number of its lot too.
    pub number: String,
    /// The recipe.
    #[model(references = "fnb_batch_recipes")]
    pub recipe_id: i64,
    /// The outlet whose kitchen makes it.
    pub outlet_id: Option<i64>,
    /// The stock location it takes the ingredients from and delivers to.
    pub location_id: i64,
    /// What the chef plans to make, in thousandths.
    pub planned_yield_milli: i64,
    /// What came out, in thousandths (entered at `finish`).
    pub actual_yield_milli: Option<i64>,
    /// The last day the output can be used (entered at `finish`).
    pub expiry_date: Option<NaiveDate>,
    /// The lot it made (`inventory.lot`).
    pub lot_id: Option<i64>,
    /// Why it is on hold.
    pub hold_reason: Option<String>,
    /// Why it was discarded.
    pub discard_reason: Option<String>,
    /// The chef who planned it.
    pub planned_by: Option<i64>,
    /// The chef who started it.
    pub started_by: Option<i64>,
    /// The chef who finished it.
    pub finished_by: Option<i64>,
    /// When it was started.
    pub started_at: Option<DateTime>,
    /// When it was finished.
    pub finished_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: BatchStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ProductionBatch {
    /// The batch as a [`Subject`]: `fnb.batch:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The lot the batch made, as a [`Subject`].
    pub fn lot(&self) -> Option<Subject> {
        self.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id))
    }

    /// The batch that made lot `lot_id`.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_lot(db: &Db, lot_id: i64) -> Result<Option<ProductionBatch>> {
        ProductionBatch::query()
            .where_(ProductionBatch::LOT_ID.eq(Some(lot_id)))
            .first(db)
            .await
    }

    /// The batches, newest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn newest(db: &Db, limit: u64) -> Result<Vec<ProductionBatch>> {
        ProductionBatch::query()
            .order_by_desc(ProductionBatch::ID)
            .limit(limit)
            .get(db)
            .await
    }

    /// The ingredient lots the batch used.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn inputs(&self, db: &Db) -> Result<Vec<BatchInput>> {
        BatchInput::query()
            .where_(BatchInput::BATCH_ID.eq(self.id))
            .order_by(BatchInput::ID)
            .get(db)
            .await
    }
}

/// An ingredient lot a batch used: one row of the lot genealogy.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_batch_inputs", index(batch_id), index(lot_id))]
pub struct BatchInput {
    /// The primary key.
    pub id: i64,
    /// The batch.
    #[model(references = "fnb_batches")]
    pub batch_id: i64,
    /// The ingredient (`inventory.item`).
    pub item_id: i64,
    /// The ingredient lot (`inventory.lot`).
    pub lot_id: i64,
    /// What went into the batch, in thousandths.
    pub quantity_milli: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BatchInput {
    /// The ingredient lot as a [`Subject`].
    pub fn lot(&self) -> Subject {
        Subject::new(LOT_SUBJECT_TYPE, self.lot_id)
    }

    /// The ingredient as a [`Subject`].
    pub fn item(&self) -> Subject {
        Subject::new(ITEM_SUBJECT_TYPE, self.item_id)
    }
}

/// How a wastage entry was posted.
pub mod posted_by {
    /// The batch posted the `wastage` movement itself (a discard from
    /// `produced`).
    pub const BATCH: &str = "batch";
    /// The discard of the lot was the posting; the entry only links it.
    pub const LOT: &str = "lot";
}

/// The wastage entry of a discarded batch: one per batch, posted once.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_batch_wastage", unique(batch_id))]
pub struct BatchWastage {
    /// The primary key.
    pub id: i64,
    /// The batch.
    #[model(references = "fnb_batches")]
    pub batch_id: i64,
    /// The lot that was wasted.
    pub lot_id: i64,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// Its value, in fils.
    pub value: i64,
    /// Why.
    pub reason: String,
    /// `batch` or `lot`, one of [`posted_by`].
    pub posted_by: String,
    /// The user who recorded it; none when the system did.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BatchWastage {
    /// The entry of `batch_id`, if it was discarded.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_batch(db: &Db, batch_id: i64) -> Result<Option<BatchWastage>> {
        BatchWastage::query()
            .where_(BatchWastage::BATCH_ID.eq(batch_id))
            .first(db)
            .await
    }
}
