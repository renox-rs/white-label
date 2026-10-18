-- Money returned to a party (the Refund base: request, approve, reject,
-- pay). Nothing is deleted; the status moves only through the machine.
CREATE TABLE "wl_payments_refunds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    -- What the refund comes from: a payment (`payments.payment`) or the
    -- party's credit balance (the party's own subject).
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- Money in fils of the currency.
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    reason TEXT NOT NULL,
    -- Set by `pay`.
    method TEXT,
    reference TEXT,
    requested_by INTEGER,
    approved_by INTEGER,
    approved_at TEXT,
    -- Why it was rejected.
    decision_reason TEXT,
    -- The outgoing payment `pay` recorded.
    payment_id INTEGER REFERENCES "wl_payments_payments" (id),
    paid_at TEXT,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_refunds_status" ON "wl_payments_refunds" (status);
CREATE INDEX "wl_payments_refunds_party" ON "wl_payments_refunds" (party_id);
CREATE INDEX "wl_payments_refunds_source" ON "wl_payments_refunds" (source_type, source_id);

-- Work the module leaves for a role, e.g. "raise a charge for AED 500.00".
-- One per kind and payment.
CREATE TABLE "wl_payments_followup_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    party_id INTEGER NOT NULL,
    payment_id INTEGER REFERENCES "wl_payments_payments" (id),
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- JSON array of the refund ids the task links.
    refund_ids TEXT NOT NULL DEFAULT '[]',
    text TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    done_by INTEGER,
    done_at TEXT,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (kind, payment_id)
);

CREATE INDEX "wl_payments_followup_tasks_open" ON "wl_payments_followup_tasks" (role, status);
