-- A manual correction of a member's points, or the closing of an account
-- that still has points, by staff (wl-sales §4.7, §5.9 step 5, SAL.ACL.07;
-- BL-SAL-24). Above the limit it waits for a manager other than the
-- requester; a rejected one writes nothing.
CREATE TABLE "wl_sales_points_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_loyalty_accounts" (id),
    -- Signed: positive adds points, negative takes them off.
    points INTEGER NOT NULL,
    reason TEXT NOT NULL,
    -- Staff closing the account: the points left are taken off with it.
    closes_account BOOLEAN NOT NULL DEFAULT FALSE,
    requested_by INTEGER NOT NULL,
    approval_request_id INTEGER,
    approved_by INTEGER,
    -- The `adjust` movement it wrote.
    movement_id INTEGER,
    -- Why it was rejected or returned.
    outcome_reason TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_points_adjustments_account" ON "wl_sales_points_adjustments" (account_id);
CREATE INDEX "wl_sales_points_adjustments_status" ON "wl_sales_points_adjustments" (status);
