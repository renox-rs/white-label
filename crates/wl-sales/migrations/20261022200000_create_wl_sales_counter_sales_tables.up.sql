-- Counter sales and the lines of a sale (wl-sales §3, §4.3, §5.4; base for
-- BL-SAL-22). A counter sale is `open` (the cart) or `completed`; its status
-- moves only through the CounterSale machine. The sale lines table is shared
-- with the sales order: one line belongs to the record named by
-- (subject_type, subject_id).
CREATE TABLE "wl_sales_sale_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `sales.counter_sale` or `sales.sales_order`.
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    line_no INTEGER NOT NULL DEFAULT 1,
    -- The catalogue item as sold (the catalogue is not built here).
    item_code TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    -- `item`; `loyalty_redemption` is added by the redemption (BL-SAL-23).
    kind TEXT NOT NULL DEFAULT 'item',
    -- Thousandths: 1000 = 1.
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    -- In fils, before VAT.
    unit_price INTEGER NOT NULL DEFAULT 0,
    discount INTEGER NOT NULL DEFAULT 0,
    vat_category TEXT NOT NULL DEFAULT 'standard',
    -- An item the loyalty programme does not count (SAL.LOY.01 exclusions).
    loyalty_excluded BOOLEAN NOT NULL DEFAULT FALSE,
    -- Sales orders: what was handed over and what was cancelled.
    fulfilled_milli INTEGER NOT NULL DEFAULT 0,
    cancelled_milli INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_sale_lines_subject" ON "wl_sales_sale_lines" (subject_type, subject_id);

CREATE TABLE "wl_sales_counter_sales" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    legal_entity_id INTEGER,
    -- The customer, when the sale names one (on account, a member).
    party_id INTEGER,
    -- The till session it was rung up on (the till lifecycle is not built).
    till_session_id INTEGER,
    cashier_user_id INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- Totals in fils, kept by every change of the lines.
    net_total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    gross_total INTEGER NOT NULL DEFAULT 0,
    -- The loyalty account attached before completion (SAL.ACL.08).
    loyalty_account_id INTEGER,
    loyalty_attached_by INTEGER,
    loyalty_attached_at TEXT,
    -- The simplified tax invoice drafted for the sale (wl-invoicing).
    invoice_id INTEGER,
    completed_at TEXT,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_sales_counter_sales_number" ON "wl_sales_counter_sales" (number);
CREATE INDEX "wl_sales_counter_sales_party" ON "wl_sales_counter_sales" (party_id);
CREATE INDEX "wl_sales_counter_sales_invoice" ON "wl_sales_counter_sales" (invoice_id);
CREATE INDEX "wl_sales_counter_sales_status" ON "wl_sales_counter_sales" (status);

-- What was taken for a completed sale, per payment method.
CREATE TABLE "wl_sales_counter_sale_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    counter_sale_id INTEGER NOT NULL REFERENCES "wl_sales_counter_sales" (id) ON DELETE CASCADE,
    -- `cash`, `card`, `account` or `other`.
    method TEXT NOT NULL,
    amount INTEGER NOT NULL,
    -- The wl-payments payment, when the till recorded one.
    payment_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_counter_sale_payments_sale" ON "wl_sales_counter_sale_payments" (counter_sale_id);
CREATE INDEX "wl_sales_counter_sale_payments_payment" ON "wl_sales_counter_sale_payments" (payment_id);
