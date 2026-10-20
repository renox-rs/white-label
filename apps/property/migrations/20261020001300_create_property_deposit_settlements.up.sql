-- The settlement statement of a deposit applied to the deductions (01 Property
-- §4.3 row "Deductions exceed the deposit", BL-PRP-17): what the deposit held,
-- what went to the unpaid rent and to the damage lines, and the invoice for
-- the damages it did not cover.
CREATE TABLE "property_deposit_settlements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The id of version 1: the lease the deposit was held for.
    root_id INTEGER NOT NULL,
    lease_id INTEGER NOT NULL REFERENCES "property_leases" (id),
    deposit_id INTEGER NOT NULL,
    -- Fils: the deposit, the unpaid rent and the damage lines it was set
    -- against, the part applied to rent, the part kept for damages, and the
    -- damages invoiced because the deposit did not cover them.
    held INTEGER NOT NULL,
    rent_due INTEGER NOT NULL DEFAULT 0,
    damages INTEGER NOT NULL DEFAULT 0,
    to_rent INTEGER NOT NULL DEFAULT 0,
    to_damages INTEGER NOT NULL DEFAULT 0,
    excess INTEGER NOT NULL DEFAULT 0,
    excess_invoice_id INTEGER,
    excess_invoice_number TEXT,
    -- The approvals (PRP.ACL.05).
    approver_id INTEGER NOT NULL,
    second_approver_id INTEGER,
    applied_by INTEGER,
    applied_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "property_deposit_settlements_deposit" ON "property_deposit_settlements" (deposit_id);
CREATE INDEX "property_deposit_settlements_root" ON "property_deposit_settlements" (root_id);
