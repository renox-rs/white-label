-- Aggregator payout statements (06 F&B §4.3, §7.1 *Payout statement*; the base
-- for BL-FNB-09 and BL-FNB-10): the file the accountant imports, its lines
-- matched to orders by the aggregator's order number, the column presets per
-- aggregator, the reason mapping of refunds, and the orders entered late
-- because the statement named them and the kitchen never recorded them.
CREATE TABLE "fnb_payout_statements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    -- The sales channel of the aggregator, lower case.
    aggregator TEXT NOT NULL,
    reference TEXT NOT NULL,
    -- The reference trimmed and lower-cased: unique per aggregator while the
    -- statement is not voided.
    reference_key TEXT NOT NULL,
    file_name TEXT NOT NULL DEFAULT '',
    preset TEXT NOT NULL DEFAULT '',
    period_from TEXT,
    period_to TEXT,
    line_count INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    commission_total INTEGER NOT NULL DEFAULT 0,
    promo_total INTEGER NOT NULL DEFAULT 0,
    refund_total INTEGER NOT NULL DEFAULT 0,
    net_total INTEGER NOT NULL DEFAULT 0,
    -- The expected payout booked as owed by the aggregator on `match`.
    receivable_amount INTEGER NOT NULL DEFAULT 0,
    receivable_booked_at TEXT,
    imported_by INTEGER,
    matched_at TEXT,
    matched_by INTEGER,
    dispute_reason TEXT,
    disputed_at TEXT,
    dispute_task_id INTEGER,
    resolve_note TEXT,
    resolved_at TEXT,
    -- The reconciliation task of `FNB.RUL.10`.
    task_id INTEGER,
    settled_at TEXT,
    void_reason TEXT,
    voided_at TEXT,
    voided_by INTEGER,
    status TEXT NOT NULL DEFAULT 'imported',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_payout_statements_number" ON "fnb_payout_statements" (number);
CREATE INDEX "fnb_payout_statements_status" ON "fnb_payout_statements" (status);
CREATE INDEX "fnb_payout_statements_aggregator" ON "fnb_payout_statements" (aggregator);
-- A reference is used once per aggregator, unless the statement was voided.
CREATE UNIQUE INDEX "fnb_payout_statements_reference" ON "fnb_payout_statements" (aggregator, reference_key) WHERE status <> 'voided';

CREATE TABLE "fnb_payout_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    statement_id INTEGER NOT NULL REFERENCES "fnb_payout_statements" (id),
    line_no INTEGER NOT NULL,
    -- The aggregator's own order number as on the file, and trimmed lower case.
    order_number TEXT NOT NULL,
    order_key TEXT NOT NULL,
    gross INTEGER NOT NULL DEFAULT 0,
    commission INTEGER NOT NULL DEFAULT 0,
    promo INTEGER NOT NULL DEFAULT 0,
    refund INTEGER NOT NULL DEFAULT 0,
    net INTEGER NOT NULL DEFAULT 0,
    -- The aggregator's refund reason as printed, the category and the party
    -- proposed by the mapping or set by the accountant.
    reason TEXT NOT NULL DEFAULT '',
    category TEXT,
    party TEXT,
    sales_order_id INTEGER,
    order_id INTEGER,
    late_order_id INTEGER,
    -- `matched`, `unmatched` or `explained`.
    match_state TEXT NOT NULL DEFAULT 'unmatched',
    explanation TEXT,
    -- The sales return of a refund line (kind *no goods*, refunded by the
    -- channel) and when it was taken back.
    return_id INTEGER,
    return_reversed_at TEXT,
    disputed INTEGER NOT NULL DEFAULT 0,
    dispute_note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_payout_lines_statement" ON "fnb_payout_lines" (statement_id);
CREATE INDEX "fnb_payout_lines_order" ON "fnb_payout_lines" (order_key);

-- The reason mapping of an aggregator: its refund reason text to a complaint
-- category and a proposed responsible party.
CREATE TABLE "fnb_payout_reasons" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    aggregator TEXT NOT NULL,
    reason_key TEXT NOT NULL,
    category TEXT NOT NULL,
    party TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_payout_reasons_key" ON "fnb_payout_reasons" (aggregator, reason_key);

-- The column mapping preset of an aggregator's file: which header holds what.
CREATE TABLE "fnb_import_presets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    aggregator TEXT NOT NULL,
    name TEXT NOT NULL,
    delimiter TEXT NOT NULL DEFAULT ',',
    order_column TEXT NOT NULL,
    gross_column TEXT NOT NULL DEFAULT '',
    commission_column TEXT NOT NULL DEFAULT '',
    promo_column TEXT NOT NULL DEFAULT '',
    refund_column TEXT NOT NULL DEFAULT '',
    net_column TEXT NOT NULL DEFAULT '',
    reason_column TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_import_presets_aggregator" ON "fnb_import_presets" (aggregator);

-- An order the statement named and the kitchen never recorded, entered by the
-- cashier afterwards as a late entry (`PLT.ACL.10`).
CREATE TABLE "fnb_late_orders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    aggregator TEXT NOT NULL,
    order_number TEXT NOT NULL,
    order_key TEXT NOT NULL,
    -- When the order really happened (the business time).
    occurred_at TEXT NOT NULL,
    entered_at TEXT,
    late INTEGER NOT NULL DEFAULT 0,
    late_reason TEXT,
    late_entry_id INTEGER,
    gross INTEGER NOT NULL DEFAULT 0,
    entered_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_late_orders_key" ON "fnb_late_orders" (aggregator, order_key);

-- The payout reconciliation reads what the cancellation flagged (BL-FNB-04,
-- BL-FNB-05): the statement that paid the compensation or refunded the line.
ALTER TABLE "fnb_orders" ADD COLUMN compensation_statement_id INTEGER;
ALTER TABLE "fnb_orders" ADD COLUMN compensation_matched_at TEXT;
ALTER TABLE "fnb_expected_refunds" ADD COLUMN statement_id INTEGER;
