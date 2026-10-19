-- What budget vs actual maps to a budget category (wl-invoicing §5.15 step 2,
-- `FIN.BUD.01`; issue #89): the charge type of an invoice line and the cost
-- code of a supplier bill line. A line without one is reported as
-- *Uncategorised*.
ALTER TABLE "wl_invoicing_invoice_lines" ADD COLUMN charge_type TEXT;
ALTER TABLE "wl_invoicing_supplier_bill_lines" ADD COLUMN cost_code TEXT;

CREATE INDEX "wl_invoicing_invoice_lines_charge_type"
    ON "wl_invoicing_invoice_lines" (charge_type);
CREATE INDEX "wl_invoicing_supplier_bill_lines_cost_code"
    ON "wl_invoicing_supplier_bill_lines" (cost_code);

-- The warnings of `FIN.RUL.24` already sent: once per entity, category and
-- month.
CREATE TABLE "wl_invoicing_budget_warnings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    category_id INTEGER NOT NULL REFERENCES "wl_invoicing_budget_categories" (id),
    -- The first day of the month.
    month TEXT NOT NULL,
    -- The approved line and the actual when the warning went out, in fils.
    budget INTEGER NOT NULL,
    actual INTEGER NOT NULL,
    sent_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_budget_warnings_once"
    ON "wl_invoicing_budget_warnings" (legal_entity_id, category_id, month);
