-- The review of a supplier whose monthly score is under the threshold
-- (wl-inventory §5.9 steps 4-5, `STK.RUL.09`, issue #249).
CREATE TABLE "wl_inventory_supplier_reviews" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    supplier_party_id INTEGER NOT NULL,
    -- `open` or `closed`.
    status TEXT NOT NULL DEFAULT 'open',
    -- The snapshots added to the review, as JSON ids, oldest first.
    snapshot_ids TEXT NOT NULL DEFAULT '[]',
    -- `keep`, `watch`, `change_preferred` or `request_block`.
    decision TEXT,
    decided_by INTEGER,
    decided_at TEXT,
    -- The items a change of the preferred supplier was asked for, as JSON ids.
    decision_items TEXT NOT NULL DEFAULT '[]',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_supplier_reviews_supplier"
    ON "wl_inventory_supplier_reviews" (supplier_party_id);
