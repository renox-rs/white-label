-- Landed costs (wl-inventory §3, §4 *Landed cost*): import costs spread over
-- received items, and the adjustments that follow a late or corrected cost.
CREATE TABLE "wl_inventory_landed_costs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT UNIQUE,
    legal_entity_id INTEGER NOT NULL,
    -- `value`, `quantity`, `weight` or `volume`.
    basis TEXT NOT NULL DEFAULT 'value',
    status TEXT NOT NULL DEFAULT 'draft',
    -- Whether `LandedCostAllocated` was sent and the variance posted.
    announced INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    allocated_by INTEGER,
    allocated_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_landed_costs_status" ON "wl_inventory_landed_costs" (status);

CREATE TABLE "wl_inventory_landed_cost_receipts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    landed_cost_id INTEGER NOT NULL REFERENCES "wl_inventory_landed_costs" (id) ON DELETE CASCADE,
    goods_receipt_id INTEGER NOT NULL REFERENCES "wl_inventory_goods_receipts" (id),
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_landed_cost_receipts_receipt"
    ON "wl_inventory_landed_cost_receipts" (landed_cost_id, goods_receipt_id);

-- An adjustment of an allocated landed cost: a late or corrected cost.
CREATE TABLE "wl_inventory_landed_cost_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    landed_cost_id INTEGER NOT NULL REFERENCES "wl_inventory_landed_costs" (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'draft',
    -- The supplier bill that caused it, in wl-invoicing, if any.
    bill_id INTEGER,
    -- The supplier bill's reference.
    bill_reference TEXT NOT NULL DEFAULT '',
    posting_date TEXT NOT NULL,
    -- The sum of its cost lines, in fils; negative for a refund.
    total INTEGER NOT NULL DEFAULT 0,
    -- Whether `LandedCostAdjusted` was sent and the variance posted.
    announced INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    posted_by INTEGER,
    posted_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_landed_cost_adjustments_landed_cost"
    ON "wl_inventory_landed_cost_adjustments" (landed_cost_id);
CREATE UNIQUE INDEX "wl_inventory_landed_cost_adjustments_bill"
    ON "wl_inventory_landed_cost_adjustments" (landed_cost_id, bill_id);

-- Cost lines: those of the allocation (no adjustment) and those of each
-- adjustment.
CREATE TABLE "wl_inventory_landed_cost_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    landed_cost_id INTEGER NOT NULL REFERENCES "wl_inventory_landed_costs" (id) ON DELETE CASCADE,
    adjustment_id INTEGER REFERENCES "wl_inventory_landed_cost_adjustments" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    -- `freight`, `duty`, `clearing` or `insurance`.
    kind TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- In fils; negative for a refund or a credit.
    amount INTEGER NOT NULL,
    -- The supplier bill line, in wl-invoicing, if any.
    bill_line_id INTEGER,
    bill_reference TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_landed_cost_lines_landed_cost"
    ON "wl_inventory_landed_cost_lines" (landed_cost_id);

-- How the cost was spread: one row per accepted receipt line, with the
-- weight of the basis, so that an adjustment spreads on the same basis.
CREATE TABLE "wl_inventory_landed_cost_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    landed_cost_id INTEGER NOT NULL REFERENCES "wl_inventory_landed_costs" (id) ON DELETE CASCADE,
    receipt_line_id INTEGER NOT NULL REFERENCES "wl_inventory_goods_receipt_lines" (id),
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- The cost object charged on receipt (the project of a site store).
    cost_object_type TEXT,
    cost_object_id INTEGER,
    -- What the receipt line accepted, in thousandths.
    quantity_milli INTEGER NOT NULL,
    -- The weight of the line on the basis (value in fils, quantity in
    -- thousandths, or the weight or volume given).
    weight INTEGER NOT NULL,
    -- What the allocation put on the line, in fils.
    amount INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_landed_cost_allocations_landed_cost"
    ON "wl_inventory_landed_cost_allocations" (landed_cost_id);

-- What the allocation or an adjustment put on an item at a location: the
-- part that re-valued stock on hand and the part on goods already consumed.
CREATE TABLE "wl_inventory_landed_cost_shares" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    landed_cost_id INTEGER NOT NULL REFERENCES "wl_inventory_landed_costs" (id) ON DELETE CASCADE,
    adjustment_id INTEGER REFERENCES "wl_inventory_landed_cost_adjustments" (id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    cost_object_type TEXT,
    cost_object_id INTEGER,
    amount INTEGER NOT NULL,
    revalued INTEGER NOT NULL,
    variance INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_landed_cost_shares_landed_cost"
    ON "wl_inventory_landed_cost_shares" (landed_cost_id, adjustment_id);
