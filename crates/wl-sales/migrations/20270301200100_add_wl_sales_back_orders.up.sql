-- Back-orders (wl-sales §4.2 *Back-orders*, issue #136): the location whose
-- stock an order reserves from, the quantity of a line reserved and
-- back-ordered, and what each arrival of stock served (one row per arrival
-- and line, so a pass runs once per arrival and a reversed receipt can give
-- its reservations back).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN location_id INTEGER;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN reserved_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sale_lines" ADD COLUMN backordered_milli INTEGER NOT NULL DEFAULT 0;

CREATE TABLE "wl_sales_back_order_serves" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    item_code TEXT NOT NULL,
    order_id INTEGER NOT NULL,
    line_id INTEGER NOT NULL,
    quantity_milli INTEGER NOT NULL,
    reversed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_back_order_serves_line"
    ON "wl_sales_back_order_serves" (source_type, source_id, line_id);
CREATE INDEX "wl_sales_back_order_serves_order" ON "wl_sales_back_order_serves" (order_id);
