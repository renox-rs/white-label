-- The request to clear one source of a customer account's hold (wl-sales
-- §4.2 Account hold, BL-SAL-12): made by a holder of credit.release, approved
-- by another one through the kernel's approvals.
CREATE TABLE "wl_sales_account_hold_clearings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_customer_accounts" (id) ON DELETE CASCADE,
    -- The source it clears (on every account that got it).
    hold_source_id INTEGER NOT NULL REFERENCES "wl_sales_account_hold_sources" (id),
    -- `cheque_replaced`, `balance_paid` or `other`.
    reason_kind TEXT NOT NULL,
    note TEXT,
    requested_by INTEGER NOT NULL,
    approval_request_id INTEGER,
    approved_by INTEGER,
    outcome_reason TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_account_hold_clearings_account" ON "wl_sales_account_hold_clearings" (account_id);
CREATE INDEX "wl_sales_account_hold_clearings_status" ON "wl_sales_account_hold_clearings" (status);
