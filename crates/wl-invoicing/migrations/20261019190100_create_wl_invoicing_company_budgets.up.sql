-- One version of the budget of one legal entity for one fiscal year
-- (wl-invoicing §3 *Company budget*, §4; issue #88). The status moves only
-- through the machine.
CREATE TABLE "wl_invoicing_company_budgets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- Named by the calendar year of the first month.
    fiscal_year INTEGER NOT NULL,
    -- The month the year starts in (the setting when the version was made).
    start_month INTEGER NOT NULL DEFAULT 1,
    version INTEGER NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    -- How the version started: empty, last_budget, last_actual or revision.
    start_choice TEXT NOT NULL DEFAULT 'empty',
    prepared_by INTEGER,
    submitted_by INTEGER,
    submitted_at TEXT,
    approved_by INTEGER,
    approved_at TEXT,
    approval_request_id INTEGER,
    -- The earlier approved version this one replaced.
    supersedes_id INTEGER,
    revision_reason TEXT,
    return_reason TEXT,
    discard_reason TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_company_budgets_version"
    ON "wl_invoicing_company_budgets" (legal_entity_id, fiscal_year, version);
CREATE INDEX "wl_invoicing_company_budgets_status"
    ON "wl_invoicing_company_budgets" (status);
-- At most one version of an entity and year is being prepared or waiting for
-- its approval.
CREATE UNIQUE INDEX "wl_invoicing_company_budgets_open"
    ON "wl_invoicing_company_budgets" (legal_entity_id, fiscal_year)
    WHERE status IN ('draft', 'submitted');

-- The budgeted amount for one category and month, in fils of AED without VAT.
CREATE TABLE "wl_invoicing_company_budget_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    budget_id INTEGER NOT NULL REFERENCES "wl_invoicing_company_budgets" (id),
    category_id INTEGER NOT NULL REFERENCES "wl_invoicing_budget_categories" (id),
    -- The first day of the month.
    month TEXT NOT NULL,
    amount INTEGER NOT NULL,
    note TEXT,
    -- A month that had ended when the version was created.
    locked INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_company_budget_lines_cell"
    ON "wl_invoicing_company_budget_lines" (budget_id, category_id, month);
CREATE INDEX "wl_invoicing_company_budget_lines_category"
    ON "wl_invoicing_company_budget_lines" (category_id);

-- The reminders of `FIN.RUL.26` already sent, so each goes out once.
CREATE TABLE "wl_invoicing_budget_reminders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    fiscal_year INTEGER NOT NULL,
    -- advance (N days before the year) or first_day.
    kind TEXT NOT NULL,
    sent_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_budget_reminders_once"
    ON "wl_invoicing_budget_reminders" (legal_entity_id, fiscal_year, kind);
