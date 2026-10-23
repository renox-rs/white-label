-- The per-customer cash on delivery flag the checkout reads (06 F&B §7.2
-- `FNB.RUL.14`; BL-FNB-03): turned off after repeated customer-side failures,
-- turned back on by the kitchen manager; failures before the last time it was
-- turned on are not counted.
CREATE TABLE "fnb_customer_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    cod_allowed INTEGER NOT NULL DEFAULT 1,
    -- When the rule turned it off, and on the failure of which order.
    cod_off_at TEXT,
    cod_off_order_id INTEGER,
    -- When a kitchen manager last turned it on, and who.
    cod_on_at TEXT,
    cod_on_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_customer_flags_party" ON "fnb_customer_flags" (party_id);
