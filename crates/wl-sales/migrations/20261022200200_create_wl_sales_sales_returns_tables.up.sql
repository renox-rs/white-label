-- Sales returns (wl-sales §3, §4.5, §5.6; base for BL-SAL-22): goods that
-- come back, or a refund after completion with nothing to return. The status
-- moves only through the SalesReturn machine.
CREATE TABLE "wl_sales_sales_returns" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    -- The sale returned: `sales.counter_sale` or `sales.sales_order`.
    sale_type TEXT NOT NULL,
    sale_id INTEGER NOT NULL,
    party_id INTEGER,
    -- `goods` or `no_goods`.
    kind TEXT NOT NULL DEFAULT 'goods',
    -- `credit_note`, `refund`, `replacement` or `refunded_by_channel`.
    resolution TEXT,
    reason TEXT NOT NULL DEFAULT '',
    -- The value returned, in fils: before VAT, VAT, with VAT.
    net_value INTEGER NOT NULL DEFAULT 0,
    vat_value INTEGER NOT NULL DEFAULT 0,
    gross_value INTEGER NOT NULL DEFAULT 0,
    -- The part of net_value the loyalty programme counts (SAL.RUL.07).
    eligible_net INTEGER NOT NULL DEFAULT 0,
    -- The credit note issued for it in wl-invoicing.
    credit_note_id INTEGER,
    complaint_category TEXT,
    responsible_party TEXT,
    -- Why it was rejected or reversed.
    closing_reason TEXT,
    requested_by INTEGER,
    handled_by INTEGER,
    received_at TEXT,
    resolved_at TEXT,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_sales_returns_number" ON "wl_sales_sales_returns" (number);
CREATE INDEX "wl_sales_sales_returns_sale" ON "wl_sales_sales_returns" (sale_type, sale_id);
CREATE INDEX "wl_sales_sales_returns_credit_note" ON "wl_sales_sales_returns" (credit_note_id);
CREATE INDEX "wl_sales_sales_returns_status" ON "wl_sales_sales_returns" (status);

-- The quantity of one sale line a return takes back.
CREATE TABLE "wl_sales_sales_return_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    return_id INTEGER NOT NULL REFERENCES "wl_sales_sales_returns" (id) ON DELETE CASCADE,
    sale_line_id INTEGER NOT NULL,
    quantity_milli INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_sales_return_lines_return" ON "wl_sales_sales_return_lines" (return_id);
CREATE INDEX "wl_sales_sales_return_lines_line" ON "wl_sales_sales_return_lines" (sale_line_id);
