-- 04 Accounting: the firm's switches, kept as text (BL-ACC-22). `ACC.RUL.07`,
-- the suspension of an engagement for an invoice overdue by N days, is off
-- until the firm turns it on.
CREATE TABLE "accounting_settings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "accounting_settings_key" ON "accounting_settings" (key);
