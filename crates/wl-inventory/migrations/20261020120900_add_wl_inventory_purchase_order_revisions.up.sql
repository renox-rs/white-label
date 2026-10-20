-- Revising an issued purchase order (wl-inventory §4 *Purchase order*
-- `revise`): who asked for a later date, the PDF in force, and the earlier
-- revisions kept read-only.
ALTER TABLE "wl_inventory_purchase_orders" ADD COLUMN date_requested_by TEXT;
-- The PDF document of the revision in force (ASCII text).
ALTER TABLE "wl_inventory_purchase_orders" ADD COLUMN document TEXT;

CREATE TABLE "wl_inventory_purchase_order_revisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    purchase_order_id INTEGER NOT NULL REFERENCES "wl_inventory_purchase_orders" (id) ON DELETE CASCADE,
    -- The revision number this row keeps (the one that was replaced).
    revision INTEGER NOT NULL,
    -- Why the revision was replaced.
    reason TEXT NOT NULL,
    -- The total of the revision, in minor units of the order's currency.
    total INTEGER NOT NULL,
    expected_date TEXT,
    date_requested_by TEXT,
    -- The lines of the revision, as JSON.
    lines TEXT NOT NULL,
    -- The PDF document of the revision.
    document TEXT,
    revised_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_purchase_order_revisions_revision"
    ON "wl_inventory_purchase_order_revisions" (purchase_order_id, revision);
