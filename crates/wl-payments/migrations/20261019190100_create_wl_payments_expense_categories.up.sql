-- Expense categories (wl-payments §3 *Expense category*, §7; issue #119): a
-- kind of spending with its VAT treatment and, for the budget, the budget
-- category it counts in. The seeded *Owner drawings* category has VAT blocked,
-- allows no cost object and maps to the budget category of kind
-- `owner_drawings`.
CREATE TABLE "wl_payments_expense_categories" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    -- `recoverable`, `blocked` or `no_vat`.
    vat_treatment TEXT NOT NULL DEFAULT 'recoverable',
    -- The code of the wl-invoicing budget category (`FIN.BUD.01`), if any.
    budget_category TEXT,
    -- Whether a line may be posted to a cost object.
    cost_object_allowed INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_payments_expense_categories_code"
    ON "wl_payments_expense_categories" (code);
