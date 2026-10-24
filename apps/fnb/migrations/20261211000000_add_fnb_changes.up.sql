-- Changes to a placed or accepted order (06 F&B §7.1 `amend`, `cancel_line`;
-- BL-FNB-05): who changed what and why (the "customer agreed" tick with the
-- time of the call, the reason for a cancelled line, the approval that
-- waits), and the refund an aggregator is expected to make for a line
-- cancelled on its order (the payout reconciliation reads it).
CREATE TABLE "fnb_order_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "fnb_orders" (id),
    -- `amend` or `cancel_line`.
    kind TEXT NOT NULL,
    -- The sale line cancelled, or the line the amendment replaced.
    sale_line_id INTEGER,
    reason_code TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT '',
    customer_agreed INTEGER NOT NULL DEFAULT 0,
    -- The time of the call to the customer, HH:MM.
    call_time TEXT,
    requested_by INTEGER,
    -- The approval request while a manager's yes is awaited.
    approval_request_id INTEGER,
    -- `done` or `waiting`.
    status TEXT NOT NULL DEFAULT 'done',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_order_changes_order" ON "fnb_order_changes" (order_id);

CREATE TABLE "fnb_expected_refunds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id INTEGER NOT NULL REFERENCES "fnb_orders" (id),
    sale_line_id INTEGER,
    -- What the aggregator is expected to refund, with VAT, in fils.
    amount INTEGER NOT NULL DEFAULT 0,
    reason TEXT NOT NULL DEFAULT '',
    -- When the payout statement matched it (BL-FNB-10).
    matched_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_expected_refunds_order" ON "fnb_expected_refunds" (order_id);
