-- Credit control of sales orders (wl-sales §3 Customer account, Credit group,
-- Credit check; §4.2 credit_hold, hold; §6 SAL.ACL.03, SAL.RUL.01; BL-SAL-10).

-- Companies under one owner that share a combined limit (PTY.REL.01). The
-- group is named here: wl-parties exposes no group membership to the kernel
-- yet.
CREATE TABLE "wl_sales_credit_groups" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The owner of the companies.
    party_id INTEGER NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    -- The combined limit in fils; NULL: none.
    combined_limit INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_credit_groups_party" ON "wl_sales_credit_groups" (party_id);

-- The commercial terms of a buying party.
CREATE TABLE "wl_sales_customer_accounts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    -- The credit limit in fils; NULL: no limit.
    credit_limit INTEGER,
    -- Payment terms in days.
    payment_terms_days INTEGER NOT NULL DEFAULT 30,
    -- Days past terms before an overdue invoice fails the check; NULL: the
    -- module setting (30).
    grace_days INTEGER,
    credit_group_id INTEGER REFERENCES "wl_sales_credit_groups" (id),
    -- The user who sells to the customer and is told of a hold.
    sales_rep_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_customer_accounts_party" ON "wl_sales_customer_accounts" (party_id);
CREATE INDEX "wl_sales_customer_accounts_group" ON "wl_sales_customer_accounts" (credit_group_id);

-- Why an account is on hold: one row per open source (a bounced cheque). The
-- flag is set while any row is open (SAL.RUL.05, BL-SAL-12).
CREATE TABLE "wl_sales_account_hold_sources" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES "wl_sales_customer_accounts" (id) ON DELETE CASCADE,
    -- The subject that caused it, e.g. `payments.cheque` 12.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- `bounced_cheque`.
    reason TEXT NOT NULL DEFAULT 'bounced_cheque',
    cleared_at TEXT,
    cleared_by INTEGER,
    clearing_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_account_hold_sources_source" ON "wl_sales_account_hold_sources" (account_id, source_type, source_id);

-- What an order on hold remembers (BL-SAL-10).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN credit_gate INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN state_before_hold TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN hold_reason TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN hold_open_balance INTEGER;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN hold_overdue INTEGER;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN hold_limit INTEGER;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN held_at TEXT;
-- What a release remembers, so the next dispatch does not check again while
-- the customer owes no more (BL-SAL-11).
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN released_at TEXT;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN release_exposure INTEGER;
ALTER TABLE "wl_sales_sales_orders" ADD COLUMN release_overdue INTEGER;
