-- Sales orders and what was handed over (wl-sales §3, §4.2, §5.2, §5.3; base
-- for BL-SAL-22). The status moves only through the SalesOrder machine. The
-- order's lines are in "wl_sales_sale_lines" (subject sales.sales_order).
CREATE TABLE "wl_sales_sales_orders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    party_id INTEGER NOT NULL,
    legal_entity_id INTEGER,
    -- `counter`, `phone`, `website`, `portal`, `aggregator`... (§7 channels).
    channel TEXT NOT NULL DEFAULT 'phone',
    -- `on_account`, `online`, `cash_on_delivery`, `cash` or `card`.
    payment_mode TEXT NOT NULL DEFAULT 'on_account',
    currency TEXT NOT NULL DEFAULT 'AED',
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    created_by INTEGER,
    -- The loyalty account attached before the order is fulfilled.
    loyalty_account_id INTEGER,
    loyalty_attached_by INTEGER,
    loyalty_attached_at TEXT,
    -- The invoice drafted when the order was fulfilled (wl-invoicing).
    invoice_id INTEGER,
    -- Why it was cancelled or rejected.
    reason TEXT,
    confirmed_at TEXT,
    fulfilled_at TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_sales_orders_number" ON "wl_sales_sales_orders" (number);
CREATE INDEX "wl_sales_sales_orders_party" ON "wl_sales_sales_orders" (party_id);
CREATE INDEX "wl_sales_sales_orders_invoice" ON "wl_sales_sales_orders" (invoice_id);
CREATE INDEX "wl_sales_sales_orders_status" ON "wl_sales_sales_orders" (status);

-- One hand-over of an order's goods: a dispatch note, a delivery, a
-- collection. A source fulfils an order once (idempotent per source).
CREATE TABLE "wl_sales_order_fulfilments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "wl_sales_sales_orders" (id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- JSON: [{"line": 1, "quantity_milli": 2000}], what the source still holds.
    lines TEXT NOT NULL DEFAULT '[]',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_order_fulfilments_source" ON "wl_sales_order_fulfilments" (order_id, source_type, source_id);
