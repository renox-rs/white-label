-- A package whose payment was reversed (BL-SAL-16, wl-sales §4.4 "Payment
-- reversed after activation"). The package keeps its state; it remembers the
-- reopened balance, the seller's collect task and what was refused meanwhile.
ALTER TABLE "wl_sales_packages" ADD COLUMN reopened_balance INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_packages" ADD COLUMN reopened_at TEXT;
ALTER TABLE "wl_sales_packages" ADD COLUMN collect_ticket_id INTEGER;

-- One row per reversal that reopened a package's invoice, so an event that is
-- delivered twice opens one task and one notice.
CREATE TABLE "wl_sales_package_reopenings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL REFERENCES "wl_sales_packages" (id) ON DELETE CASCADE,
    -- The record the money came from, e.g. `payments.payment:42`.
    money_source TEXT NOT NULL,
    invoice_id INTEGER,
    -- The balance after the reopening, in fils.
    balance INTEGER NOT NULL DEFAULT 0,
    ticket_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_package_reopenings_source" ON "wl_sales_package_reopenings" (package_id, money_source);

-- A consumption refused while consumption is suspended for an unpaid
-- reopened balance. The seller's task lists them for a manual `consume`.
CREATE TABLE "wl_sales_package_refusals" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL REFERENCES "wl_sales_packages" (id) ON DELETE CASCADE,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    quantity INTEGER NOT NULL DEFAULT 1,
    -- `unpaid`.
    reason TEXT NOT NULL DEFAULT 'unpaid',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_package_refusals_package" ON "wl_sales_package_refusals" (package_id);
