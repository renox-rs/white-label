-- The payer's money side (#80, wl-invoicing §3 *Remittance advice* and
-- *Payer credit balance*): the remittance advice recorded on a payer bill,
-- the part of each payment allocated to a bill line, the payer's credit
-- balance as signed movements, and the recoupments.
CREATE TABLE "wl_invoicing_payer_remittances" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_bill_id INTEGER NOT NULL REFERENCES "wl_invoicing_payer_bills" (id),
    payer_party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    -- The payer's reference; unique per payer.
    reference TEXT NOT NULL,
    advice_date TEXT NOT NULL,
    -- The wl-payments payment that carried the money, if recorded there.
    payment_id INTEGER,
    paid_total INTEGER NOT NULL DEFAULT 0,
    denied_total INTEGER NOT NULL DEFAULT 0,
    -- The part moved to the payer credit balance (approved).
    overpayment INTEGER NOT NULL DEFAULT 0,
    -- The lines as the payer sent them (JSON).
    lines TEXT NOT NULL,
    approval_request_id INTEGER,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_payer_remittances_reference"
    ON "wl_invoicing_payer_remittances" (payer_party_id, reference);
CREATE INDEX "wl_invoicing_payer_remittances_bill"
    ON "wl_invoicing_payer_remittances" (payer_bill_id);

-- Money of a remittance applied to one payer bill line.
CREATE TABLE "wl_invoicing_payer_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_bill_id INTEGER NOT NULL REFERENCES "wl_invoicing_payer_bills" (id),
    payer_bill_line_id INTEGER NOT NULL
        REFERENCES "wl_invoicing_payer_bill_lines" (id),
    remittance_id INTEGER NOT NULL REFERENCES "wl_invoicing_payer_remittances" (id),
    payment_id INTEGER,
    amount INTEGER NOT NULL,
    removed_at TEXT,
    removed_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_payer_allocations_bill"
    ON "wl_invoicing_payer_allocations" (payer_bill_id);
CREATE INDEX "wl_invoicing_payer_allocations_payment"
    ON "wl_invoicing_payer_allocations" (payment_id);

-- Money taken back by the payer.
CREATE TABLE "wl_invoicing_payer_recoupments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_bill_id INTEGER NOT NULL REFERENCES "wl_invoicing_payer_bills" (id),
    payer_party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    code TEXT NOT NULL,
    reason TEXT,
    -- The later remittance that carried it.
    carrier_reference TEXT,
    carrier_date TEXT,
    amount INTEGER NOT NULL,
    -- Set off from the payer credit balance first.
    from_credit INTEGER NOT NULL DEFAULT 0,
    -- Deducted from the remittance that carried it.
    deducted INTEGER NOT NULL DEFAULT 0,
    lines TEXT NOT NULL,
    -- A reversed payment caused it (no approval).
    reversal BOOLEAN NOT NULL DEFAULT FALSE,
    payment_id INTEGER,
    approval_request_id INTEGER,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_payer_recoupments_bill"
    ON "wl_invoicing_payer_recoupments" (payer_bill_id);

-- The payer credit balance: the sum of the signed movements.
CREATE TABLE "wl_invoicing_payer_credit_movements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- `overpayment` (adds), `set_off` (a recoupment took it) or `refund`.
    kind TEXT NOT NULL,
    -- Signed: above 0 adds to the balance.
    amount INTEGER NOT NULL,
    payer_bill_id INTEGER,
    remittance_id INTEGER,
    recoupment_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_payer_credit_movements_payer"
    ON "wl_invoicing_payer_credit_movements" (payer_party_id, legal_entity_id, currency);
