-- The purchase price variance of a receipt that covers a negative balance
-- (BL-STK-19, #238): at a location set to `allow_consumption` automatic
-- consumption went below zero at the average cost; the receipt that covers it
-- values that quantity at its own cost, and the difference is kept here until
-- it is posted.
CREATE TABLE "wl_inventory_price_variances" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    receipt_id INTEGER NOT NULL REFERENCES "wl_inventory_goods_receipts" (id),
    receipt_line_id INTEGER NOT NULL,
    -- The negative quantity the receipt covered, in thousandths.
    quantity_milli INTEGER NOT NULL,
    -- The cost the consumption used, and the cost of the receipt (fils a unit).
    cost_used INTEGER NOT NULL,
    receipt_cost INTEGER NOT NULL,
    -- (receipt cost - cost used) x quantity, in fils.
    amount INTEGER NOT NULL,
    posted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_inventory_price_variances_receipt" ON "wl_inventory_price_variances" (receipt_id);
