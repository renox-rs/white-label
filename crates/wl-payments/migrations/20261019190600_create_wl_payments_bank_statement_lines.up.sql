-- One bank transaction of an import (wl-payments §3 *Statement line*; issues
-- #118, #119). Created by `parse`, removed by `revert`.
CREATE TABLE "wl_payments_bank_statement_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    import_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    bank_account TEXT NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    line_date TEXT NOT NULL,
    -- In fils: above 0 is money in, below 0 money out.
    amount INTEGER NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    reference TEXT NOT NULL DEFAULT '',
    counterparty TEXT NOT NULL DEFAULT '',
    -- `unmatched`, `suggested`, `matched` or `ignored`.
    status TEXT NOT NULL DEFAULT 'unmatched',
    -- What a matched line was matched to (a payment).
    match_kind TEXT,
    match_id INTEGER,
    ignore_reason TEXT,
    -- The owner drawing request the line was marked with.
    drawing_request_id INTEGER,
    -- Why a line was put back to `unmatched` (a rejected drawing).
    reason TEXT,
    marked_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_bank_statement_lines_import"
    ON "wl_payments_bank_statement_lines" (import_id, status);
CREATE INDEX "wl_payments_bank_statement_lines_drawing"
    ON "wl_payments_bank_statement_lines" (drawing_request_id);
CREATE INDEX "wl_payments_bank_statement_lines_dedupe"
    ON "wl_payments_bank_statement_lines" (legal_entity_id, bank_account, line_date);

ALTER TABLE "wl_payments_drawing_requests" ADD COLUMN statement_line_id INTEGER;
