CREATE TABLE "wl_payments_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The receipt (in) or voucher (out) number, set by `record`; NULL while
    -- the payment is `pending`.
    number TEXT UNIQUE,
    direction TEXT NOT NULL DEFAULT 'in',
    legal_entity_id INTEGER NOT NULL,
    -- Free text until kernel bank accounts exist.
    bank_account TEXT,
    party_id INTEGER NOT NULL,
    method TEXT NOT NULL,
    -- JSON: cheque number, bank and date; card slip; transfer reference.
    method_details TEXT NOT NULL DEFAULT '{}',
    -- Money in fils of the payment's currency.
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    payment_date TEXT NOT NULL,
    reference TEXT,
    source TEXT NOT NULL DEFAULT 'manual',
    statement_line_id INTEGER,
    gateway TEXT,
    gateway_transaction TEXT,
    collected_by INTEGER,
    -- The record the money was paid in advance for, if any.
    prepaid_type TEXT,
    prepaid_id INTEGER,
    purpose TEXT NOT NULL DEFAULT 'none',
    recorded_by INTEGER,
    -- The chargeback fields (used by the chargeback flow).
    dispute_reference TEXT,
    dispute_status TEXT,
    dispute_evidence_due TEXT,
    -- The reversal fields, written by `reverse`.
    reversal_reason TEXT,
    reversal_note TEXT,
    reversed_by INTEGER,
    reversed_on TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_payments_status" ON "wl_payments_payments" (status);
CREATE INDEX "wl_payments_payments_party" ON "wl_payments_payments" (party_id);
CREATE INDEX "wl_payments_payments_prepaid" ON "wl_payments_payments" (prepaid_type, prepaid_id);

-- Money set against a target (an invoice today). Rows are never deleted: a
-- reversal fills the `removed_*` columns.
CREATE TABLE "wl_payments_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payment_id INTEGER NOT NULL REFERENCES "wl_payments_payments" (id),
    target_type TEXT NOT NULL,
    target_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    allocated_at TEXT NOT NULL,
    allocated_by INTEGER,
    removed_at TEXT,
    removed_reason TEXT,
    removed_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_allocations_payment" ON "wl_payments_allocations" (payment_id);
CREATE INDEX "wl_payments_allocations_target" ON "wl_payments_allocations" (target_type, target_id);
