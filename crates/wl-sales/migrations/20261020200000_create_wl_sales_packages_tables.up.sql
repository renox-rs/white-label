-- Prepaid packages of sessions or units (wl-sales §3 *Package*, §4.4; base
-- for BL-SAL-16) and their movements. The status moves only through the
-- Package machine; credits_used moves only with a movement.
CREATE TABLE "wl_sales_packages" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Who uses the credits.
    holder_party_id INTEGER NOT NULL,
    -- Who pays when not the holder (a guardian).
    payer_party_id INTEGER,
    -- The catalogue item the package was sold for (the catalogue is wl-sales'
    -- own later; for now its code and name as sold).
    item_code TEXT NOT NULL,
    item_name TEXT NOT NULL DEFAULT '',
    -- `session`, `hour` or `item`.
    unit TEXT NOT NULL DEFAULT 'session',
    credits_total INTEGER NOT NULL,
    credits_used INTEGER NOT NULL DEFAULT 0,
    -- In fils.
    price_paid INTEGER NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- Set when the package activates.
    valid_from TEXT,
    valid_to TEXT,
    validity_months INTEGER NOT NULL DEFAULT 6,
    extensions INTEGER NOT NULL DEFAULT 0,
    -- The invoice the package is paid with (wl-invoicing).
    invoice_id INTEGER,
    -- The staff user who sold it.
    sold_by INTEGER,
    activated_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending_payment',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_packages_holder" ON "wl_sales_packages" (holder_party_id, item_code, status);
CREATE INDEX "wl_sales_packages_invoice" ON "wl_sales_packages" (invoice_id);
CREATE INDEX "wl_sales_packages_status" ON "wl_sales_packages" (status, valid_to);

-- A change to the credit balance, with the record that caused it.
CREATE TABLE "wl_sales_package_movements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    package_id INTEGER NOT NULL REFERENCES "wl_sales_packages" (id) ON DELETE CASCADE,
    -- `consume`, `reverse`, `extend`, `adjust` or `refund`.
    kind TEXT NOT NULL,
    -- Credits, always positive; the kind gives the direction.
    quantity INTEGER NOT NULL DEFAULT 0,
    source_type TEXT,
    source_id INTEGER,
    user_id INTEGER,
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_sales_package_movements_package" ON "wl_sales_package_movements" (package_id);
CREATE INDEX "wl_sales_package_movements_source" ON "wl_sales_package_movements" (source_type, source_id);
