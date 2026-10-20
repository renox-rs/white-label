-- The IPA, the Retention and the Guarantee of a project (02 Construction §6,
-- §7.1), as far as the termination and the final account need them. Money is
-- fils.
CREATE TABLE "construction_ipas" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- `interim` or `final_account`.
    kind TEXT NOT NULL DEFAULT 'interim',
    revision INTEGER NOT NULL DEFAULT 1,
    -- The day the work is valued to (the effective date of a termination).
    valuation_on TEXT,
    claimed_amount INTEGER NOT NULL DEFAULT 0,
    -- The certified work, before deductions.
    certified_amount INTEGER,
    -- A court or arbitration decision recorded as the certificate: its
    -- reference (the judgment is attached) and the decided amount.
    decision_reference TEXT,
    decided_amount INTEGER,
    -- The tax invoice of wl-invoicing once the IPA is invoiced.
    invoice_id INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_ipas_project" ON "construction_ipas" (project_id);

CREATE TABLE "construction_retentions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- `first` or `final`.
    part TEXT NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'held',
    due_since TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "construction_retentions_part" ON "construction_retentions" (project_id, part);

CREATE TABLE "construction_guarantees" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- `advance`, `performance` or `retention_bond`.
    kind TEXT NOT NULL,
    bank TEXT NOT NULL DEFAULT '',
    amount INTEGER NOT NULL DEFAULT 0,
    expires_on TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_guarantees_project" ON "construction_guarantees" (project_id);
