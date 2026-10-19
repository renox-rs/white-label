-- A bank balance entered by hand for a bank account, used by the cash-flow
-- forecast when no newer statement balance exists (wl-invoicing §3 *Opening
-- cash balance*, issue #91). Never edited: a wrong balance is voided with a
-- reason, or replaced by a newer one; voided rows stay and are never used.
CREATE TABLE "wl_invoicing_opening_cash_balances" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL,
    -- The bank account label, as on the payments and the statements.
    bank_account TEXT NOT NULL,
    -- In AED fils; may be below 0 (an overdraft).
    amount INTEGER NOT NULL,
    -- A balance is as of the end of this day (today or earlier).
    as_of TEXT NOT NULL,
    note TEXT,
    entered_by INTEGER,
    voided_by INTEGER,
    voided_at TEXT,
    void_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_opening_cash_balances_account" ON "wl_invoicing_opening_cash_balances" (legal_entity_id, bank_account, as_of);
