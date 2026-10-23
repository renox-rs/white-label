-- The business day and the daily closing (06 F&B §4.6 step 4, §7.1 *Daily
-- closing*, §7.2 business-day cut-off; BL-FNB-15). An outlet has a cut-off
-- (minutes after midnight, Asia/Dubai; 04:00 when blank) that applies to a
-- change from the next business day only; an order carries the trading day it
-- belongs to; the closing of an outlet and day moves through its machine and is
-- linked to the wl-sales till session of the cash count.
ALTER TABLE "fnb_outlets" ADD COLUMN cutoff_minutes INTEGER;
ALTER TABLE "fnb_outlets" ADD COLUMN pending_cutoff_minutes INTEGER;
ALTER TABLE "fnb_outlets" ADD COLUMN pending_cutoff_from TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN business_date TEXT;
UPDATE "fnb_orders" SET business_date = date(placed_at, '+4 hours', '-4 hours') WHERE placed_at IS NOT NULL;
CREATE INDEX "fnb_orders_business_date" ON "fnb_orders" (outlet_id, business_date);

CREATE TABLE "fnb_daily_closings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    outlet_id INTEGER NOT NULL REFERENCES "fnb_outlets" (id),
    business_date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    -- The till session (wl-sales) of the cash count.
    till_session_id INTEGER,
    checklist_done INTEGER NOT NULL DEFAULT 0,
    expected_cash INTEGER NOT NULL DEFAULT 0,
    counted_cash INTEGER,
    variance INTEGER,
    sales_total INTEGER NOT NULL DEFAULT 0,
    orders_count INTEGER NOT NULL DEFAULT 0,
    submitted_at TEXT,
    submitted_by INTEGER,
    approved_at TEXT,
    approved_by INTEGER,
    reopened_at TEXT,
    reopened_by INTEGER,
    reopen_reason TEXT,
    reopen_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_daily_closings_day" ON "fnb_daily_closings" (outlet_id, business_date);
CREATE INDEX "fnb_daily_closings_status" ON "fnb_daily_closings" (status);
