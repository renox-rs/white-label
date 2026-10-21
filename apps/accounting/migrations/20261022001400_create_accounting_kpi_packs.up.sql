-- 04 Accounting §7.1 *KPI pack*, `ACC.KPI.01`, `ACC.RUL.06` (BL-ACC-20,
-- BL-ACC-21): a client's KPI pack per period and version, the trial balance
-- figures it was calculated from, and the client's saved account mapping.
CREATE TABLE "accounting_kpi_packs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    period_end TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    commentary TEXT,
    commentary_required BOOLEAN NOT NULL DEFAULT TRUE,
    imported_by INTEGER,
    imported_at TEXT,
    copied_from_id INTEGER,
    published_on TEXT,
    published_by INTEGER,
    withdraw_reason TEXT,
    withdrawn_on TEXT,
    withdrawn_by INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_kpi_packs_client_party_id" ON "accounting_kpi_packs" (client_party_id);
CREATE INDEX "accounting_kpi_packs_status" ON "accounting_kpi_packs" (status);
CREATE UNIQUE INDEX "accounting_kpi_packs_client_party_id_period_end_version" ON "accounting_kpi_packs" (client_party_id, period_end, version);

CREATE TABLE "accounting_kpi_figures" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pack_id INTEGER NOT NULL REFERENCES "accounting_kpi_packs" (id),
    account_code TEXT NOT NULL,
    account_name TEXT NOT NULL DEFAULT '',
    amount INTEGER NOT NULL DEFAULT 0,
    category TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_kpi_figures_pack_id" ON "accounting_kpi_figures" (pack_id);

CREATE TABLE "accounting_kpi_mappings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    account_code TEXT NOT NULL,
    category TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "accounting_kpi_mappings_client_party_id_account_code" ON "accounting_kpi_mappings" (client_party_id, account_code);
