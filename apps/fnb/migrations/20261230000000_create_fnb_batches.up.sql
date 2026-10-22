-- Production batches of the central kitchen (06 F&B §6, §7.1 *Production
-- batch*; base for BL-FNB-11): a recipe says what a batch is made of, the
-- batch records the ingredient lots it used (the lot genealogy a recall trace
-- follows) and the lot it made, and a discarded batch keeps one wastage entry.
CREATE TABLE "fnb_batch_recipes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    -- The stock item (wl-inventory) the batch makes, lot-tracked.
    output_item_id INTEGER NOT NULL,
    -- What one standard batch yields, in thousandths of the item's unit.
    yield_milli INTEGER NOT NULL DEFAULT 1000,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_batch_recipes_code" ON "fnb_batch_recipes" (code);

CREATE TABLE "fnb_batch_recipe_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recipe_id INTEGER NOT NULL REFERENCES "fnb_batch_recipes" (id),
    -- The ingredient: a lot-tracked stock item, and its code (the stock
    -- module proposes lots by code).
    item_id INTEGER NOT NULL,
    item_code TEXT NOT NULL,
    -- What one standard batch uses of it, in thousandths.
    quantity_milli INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_batch_recipe_lines_recipe" ON "fnb_batch_recipe_lines" (recipe_id);

CREATE TABLE "fnb_batches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- BAT-0001.
    number TEXT NOT NULL,
    recipe_id INTEGER NOT NULL REFERENCES "fnb_batch_recipes" (id),
    -- The outlet whose kitchen makes it, and the stock location it takes
    -- the ingredients from and delivers to.
    outlet_id INTEGER,
    location_id INTEGER NOT NULL,
    planned_yield_milli INTEGER NOT NULL,
    actual_yield_milli INTEGER,
    -- The expiry the chef entered at `finish`.
    expiry_date TEXT,
    -- The lot the batch made (`inventory.lot`).
    lot_id INTEGER,
    -- Why the batch is on hold, and why it was discarded.
    hold_reason TEXT,
    discard_reason TEXT,
    planned_by INTEGER,
    started_by INTEGER,
    finished_by INTEGER,
    started_at TEXT,
    finished_at TEXT,
    status TEXT NOT NULL DEFAULT 'planned',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_batches_number" ON "fnb_batches" (number);
CREATE INDEX "fnb_batches_status" ON "fnb_batches" (status);
CREATE INDEX "fnb_batches_lot" ON "fnb_batches" (lot_id);

-- The ingredient lots a batch used: the lot genealogy.
CREATE TABLE "fnb_batch_inputs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL REFERENCES "fnb_batches" (id),
    item_id INTEGER NOT NULL,
    lot_id INTEGER NOT NULL,
    quantity_milli INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_batch_inputs_batch" ON "fnb_batch_inputs" (batch_id);
CREATE INDEX "fnb_batch_inputs_lot" ON "fnb_batch_inputs" (lot_id);

-- The wastage entry of a discarded batch: one per batch, posted once.
CREATE TABLE "fnb_batch_wastage" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL REFERENCES "fnb_batches" (id),
    lot_id INTEGER NOT NULL,
    quantity_milli INTEGER NOT NULL,
    value INTEGER NOT NULL DEFAULT 0,
    reason TEXT NOT NULL DEFAULT '',
    -- `batch` when the batch posted the wastage movement itself, `lot` when
    -- the discard of the lot (a hold line decided *discard*) was the posting
    -- and this entry only links it.
    posted_by TEXT NOT NULL,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_batch_wastage_batch" ON "fnb_batch_wastage" (batch_id);
