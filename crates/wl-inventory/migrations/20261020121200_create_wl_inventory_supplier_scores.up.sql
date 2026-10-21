-- The supplier score (wl-inventory §3 *Supplier score*, §5.9 step 2, issue
-- #248): what the score needs from receipts and orders, the debit notes kept
-- for it, the monthly snapshots and their review notes.

-- The day the goods arrived when it is not the day of the posting: the
-- business time of a late entry, or the day posting was asked for.
ALTER TABLE "wl_inventory_goods_receipts" ADD COLUMN arrived_at TEXT;
ALTER TABLE "wl_inventory_goods_receipts" ADD COLUMN late_entry INTEGER NOT NULL DEFAULT 0;

-- When the order was issued, and when the revision in force was.
ALTER TABLE "wl_inventory_purchase_orders" ADD COLUMN issued_at TEXT;
ALTER TABLE "wl_inventory_purchase_orders" ADD COLUMN revision_issued_at TEXT;

-- The supplier debit notes the wl-invoicing events told about.
CREATE TABLE "wl_inventory_supplier_debit_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    debit_note_id INTEGER NOT NULL UNIQUE,
    supplier_party_id INTEGER NOT NULL,
    -- The total with VAT, in fils of the bill's currency.
    amount INTEGER NOT NULL,
    -- The Asia/Dubai day it was issued.
    issued_on TEXT NOT NULL,
    cancelled INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_supplier_debit_notes_supplier"
    ON "wl_inventory_supplier_debit_notes" (supplier_party_id);

-- A monthly snapshot of one supplier's performance. Derived, never edited.
CREATE TABLE "wl_inventory_supplier_scores" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    supplier_party_id INTEGER NOT NULL,
    -- Day 1 of the month the snapshot is named by.
    snapshot_date TEXT NOT NULL,
    window_from TEXT NOT NULL,
    window_to TEXT NOT NULL,
    receipts INTEGER NOT NULL DEFAULT 0,
    on_time INTEGER NOT NULL DEFAULT 0,
    -- Fractions in basis points (10000 = 100%); none when there was no data.
    on_time_bp INTEGER,
    rejected_bp INTEGER,
    received_value INTEGER NOT NULL DEFAULT 0,
    rejected_value INTEGER NOT NULL DEFAULT 0,
    incidents INTEGER NOT NULL DEFAULT 0,
    debit_notes INTEGER NOT NULL DEFAULT 0,
    debit_note_value INTEGER NOT NULL DEFAULT 0,
    -- Can be negative: a price that went down.
    price_variance_bp INTEGER,
    score INTEGER,
    -- `good`, `watch`, `below_threshold` or `not_enough_data`.
    band TEXT NOT NULL,
    computed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_supplier_scores_month"
    ON "wl_inventory_supplier_scores" (supplier_party_id, snapshot_date);

-- A review note on a snapshot: added, never changed.
CREATE TABLE "wl_inventory_supplier_score_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    supplier_score_id INTEGER NOT NULL REFERENCES "wl_inventory_supplier_scores" (id) ON DELETE CASCADE,
    note TEXT NOT NULL,
    written_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_supplier_score_notes_score"
    ON "wl_inventory_supplier_score_notes" (supplier_score_id);
