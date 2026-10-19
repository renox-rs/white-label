-- Campaign cost entries (wl-sales §4, #138 A11) and the budget alerts already
-- sent (SAL.RUL.10, A16). The status moves only through the CostEntry machine.
CREATE TABLE "wl_sales_campaign_cost_entries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    campaign_id INTEGER NOT NULL REFERENCES "wl_sales_campaigns" (id),
    entry_date TEXT NOT NULL,
    -- AED fils; negative for a credit.
    amount INTEGER NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- The supplier bill and line the cost comes from (part 6).
    supplier_bill_id INTEGER,
    supplier_bill_line_id INTEGER,
    linked_by INTEGER,
    requested_by INTEGER NOT NULL,
    approved_by INTEGER,
    approval_request_id INTEGER,
    -- Why it was rejected or voided.
    reason TEXT,
    -- The campaign's branch, copied so the approval looks for approvers there.
    scope_type TEXT,
    scope_id TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_cost_entries_campaign" ON "wl_sales_campaign_cost_entries" (campaign_id, status);
CREATE INDEX "wl_sales_cost_entries_bill" ON "wl_sales_campaign_cost_entries" (supplier_bill_id, supplier_bill_line_id);

-- One row per campaign and threshold, never deleted: a later rise is not alerted again.
CREATE TABLE "wl_sales_campaign_budget_alerts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    campaign_id INTEGER NOT NULL REFERENCES "wl_sales_campaigns" (id),
    threshold_bps INTEGER NOT NULL,
    -- The approved costs when the threshold was reached.
    reached_amount INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (campaign_id, threshold_bps)
);
