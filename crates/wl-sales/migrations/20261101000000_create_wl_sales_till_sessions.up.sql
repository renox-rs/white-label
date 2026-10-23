-- The till session (wl-sales §4.3, SAL.POS.01; base for BL-FNB-15 and the
-- till handover of BL-PAY-19): one drawer for one shift or day. Its status
-- moves only through the TillSession machine. Money is fils.
CREATE TABLE "wl_sales_till_sessions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The till (register), the holder of the cash.
    till TEXT NOT NULL,
    location TEXT,
    legal_entity_id INTEGER,
    business_date TEXT,
    cashier_user_id INTEGER NOT NULL,
    float_amount INTEGER NOT NULL DEFAULT 0,
    expected_cash INTEGER NOT NULL DEFAULT 0,
    counted_cash INTEGER,
    variance INTEGER,
    opened_at TEXT,
    closed_at TEXT,
    closed_by INTEGER,
    reviewed_at TEXT,
    reviewed_by INTEGER,
    reopen_reason TEXT,
    reopen_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_till_sessions_till" ON "wl_sales_till_sessions" (till, business_date);
CREATE INDEX "wl_sales_till_sessions_status" ON "wl_sales_till_sessions" (status);
CREATE INDEX "wl_sales_till_sessions_cashier" ON "wl_sales_till_sessions" (cashier_user_id);
