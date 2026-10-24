-- The customer's own PO number on an order, and the unique key that lets one
-- order only claim it per customer (wl-sales §3 Sales order, §4.2 Customer
-- PO number; BL-SAL-18).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN customer_po TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN call_off TEXT;

-- One row per submitted order with a PO reference while the setting *Unique
-- customer PO number per customer* is on. Cancelled and rejected orders keep
-- theirs: the number stays used. The reference and the call-off number are
-- stored trimmed and lower case.
CREATE TABLE "wl_sales_order_po_keys" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "wl_sales_sales_orders" (id) ON DELETE CASCADE,
    party_id INTEGER NOT NULL,
    po_key TEXT NOT NULL,
    call_off_key TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_order_po_keys_order" ON "wl_sales_order_po_keys" (order_id);
CREATE UNIQUE INDEX "wl_sales_order_po_keys_key" ON "wl_sales_order_po_keys" (party_id, po_key, call_off_key);
