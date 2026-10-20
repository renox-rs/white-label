-- An expense claim and its lines (wl-payments §3 *Expense claim*, *Expense
-- line*, §4; issue #90): the part of the lifecycle that routes a line in the
-- *Owner drawings* category through the drawings approval (`FIN.ACL.21`).
-- Reimbursement, receipts, limits and cost posting come with the expense
-- feature.
CREATE TABLE "wl_payments_expense_claims" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    claimant_id INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    -- `draft`, `submitted`, `approved`, `rejected`.
    status TEXT NOT NULL DEFAULT 'draft',
    -- Sum of the lines, VAT included, in AED fils.
    total INTEGER NOT NULL DEFAULT 0,
    vat_total INTEGER NOT NULL DEFAULT 0,
    -- The part of the total in owner drawings categories (VAT blocked, so
    -- included); the drawing waits for `FIN.ACL.21`.
    drawing_amount INTEGER NOT NULL DEFAULT 0,
    -- `requested`, `approved` or `rejected`; null when there is no drawing.
    drawing_status TEXT,
    drawing_self_approved INTEGER NOT NULL DEFAULT 0,
    drawing_approved_by INTEGER,
    drawing_reason TEXT,
    approved_by INTEGER,
    decision_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_expense_claims_status"
    ON "wl_payments_expense_claims" (status);

CREATE TABLE "wl_payments_expense_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    claim_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    expense_date TEXT NOT NULL,
    merchant TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- What was paid, VAT included, and the VAT in it, in AED fils.
    amount INTEGER NOT NULL,
    vat_amount INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_payments_expense_lines_position"
    ON "wl_payments_expense_lines" (claim_id, position);
