-- Arrears escalation (01 Property `PRP.RUL.10`, BL-PRP-14): one formal payment
-- demand per lease at a time, listing the invoices overdue by N days; the
-- invoices on it are frozen when it is sent. A payment plan is a note with
-- the agreed dates (no entity of its own in the spec): the rule stays quiet
-- until the latest agreed date.
CREATE TABLE "property_arrears_demands" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The id of version 1: the lease all versions belong to.
    root_id INTEGER NOT NULL,
    -- The version that was in force when the demand was made.
    lease_id INTEGER NOT NULL REFERENCES "property_leases" (id),
    created_on TEXT NOT NULL,
    -- The day the statutory notice was sent, and by whom.
    sent_on TEXT,
    sent_by INTEGER,
    -- The proof-of-delivery date: the M-day clock starts here.
    delivered_on TEXT,
    -- The day the task to open a legal case or agree a payment plan was made.
    legal_task_on TEXT,
    -- The day the demand needed nothing more.
    closed_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_arrears_demands_root" ON "property_arrears_demands" (root_id, closed_on);

CREATE TABLE "property_arrears_demand_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    demand_id INTEGER NOT NULL REFERENCES "property_arrears_demands" (id),
    invoice_id INTEGER NOT NULL,
    number TEXT,
    -- The amount still open when the invoice was listed, in fils.
    open_amount INTEGER NOT NULL DEFAULT 0,
    due_date TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_arrears_demand_invoices_demand" ON "property_arrears_demand_invoices" (demand_id);
CREATE INDEX "property_arrears_demand_invoices_invoice" ON "property_arrears_demand_invoices" (invoice_id);

CREATE TABLE "property_payment_plans" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id INTEGER NOT NULL,
    lease_id INTEGER NOT NULL REFERENCES "property_leases" (id),
    -- The agreed dates, as text.
    note TEXT NOT NULL,
    -- The latest agreed date.
    latest_on TEXT NOT NULL,
    agreed_on TEXT NOT NULL,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_payment_plans_root" ON "property_payment_plans" (root_id);
