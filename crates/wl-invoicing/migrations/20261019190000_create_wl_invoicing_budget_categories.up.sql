-- The budget categories (wl-invoicing §3 *Budget category*, `FIN.BUD.01`; issue
-- #88): the lines of the company budget and of the budget vs actual report.
CREATE TABLE "wl_invoicing_budget_categories" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    -- income, expense or owner_drawings.
    kind TEXT NOT NULL,
    -- The category this one is a part of, for subtotals.
    parent_id INTEGER REFERENCES "wl_invoicing_budget_categories" (id),
    -- The role told by `FIN.RUL.24`; none: the finance manager only.
    owner_role TEXT,
    -- The owner or partner whose drawings an `owner_drawings` category records.
    owner_party_id INTEGER,
    position INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_budget_categories_code"
    ON "wl_invoicing_budget_categories" (code);

-- What a category collects: a charge type, a catalogue item group, a cost
-- code, an expense category, payroll or a ledger export account. A mapping
-- key belongs to at most one category.
CREATE TABLE "wl_invoicing_budget_mappings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    category_id INTEGER NOT NULL REFERENCES "wl_invoicing_budget_categories" (id),
    key_kind TEXT NOT NULL,
    key_value TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_budget_mappings_key"
    ON "wl_invoicing_budget_mappings" (key_kind, key_value);
CREATE INDEX "wl_invoicing_budget_mappings_category"
    ON "wl_invoicing_budget_mappings" (category_id);
