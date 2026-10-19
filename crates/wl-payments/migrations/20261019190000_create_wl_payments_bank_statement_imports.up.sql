-- One uploaded bank statement file for one bank account (wl-payments §3 *Bank
-- statement import*, issue #118). Only what the cash-flow forecast reads is
-- kept: the period and the closing balance. The lines and the reconciliation
-- state machine come with the bank reconciliation feature.
CREATE TABLE "wl_payments_bank_statement_imports" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- The bank account label, as on the payments.
    bank_account TEXT NOT NULL,
    file_name TEXT NOT NULL,
    -- `csv`, `excel` or `mt940`.
    format TEXT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    period_start TEXT NOT NULL,
    period_end TEXT NOT NULL,
    -- In fils; from the file when it carries one, otherwise entered at
    -- upload. As of the last day of the period. Optional; may be below 0.
    closing_balance INTEGER,
    line_count INTEGER NOT NULL DEFAULT 0,
    imported_by INTEGER,
    -- `uploaded`, `in_review`, `completed`, `failed` or `reverted`.
    status TEXT NOT NULL DEFAULT 'uploaded',
    completed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_bank_statement_imports_account" ON "wl_payments_bank_statement_imports" (legal_entity_id, bank_account, status);
