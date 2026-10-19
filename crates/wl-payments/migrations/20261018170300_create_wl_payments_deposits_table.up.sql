-- Money held as security for a source agreement (the Deposit base). The
-- status moves only through the machine; nothing is deleted.
CREATE TABLE "wl_payments_deposits" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL,
    -- What the deposit is held for (e.g. a lease), as a subject.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    -- Money in fils of the currency: the amount the source requires.
    amount INTEGER NOT NULL,
    -- What is held: the money allocated to the deposit.
    held_amount INTEGER NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'AED',
    -- The payment the deposit was received with.
    received_payment_id INTEGER REFERENCES "wl_payments_payments" (id),
    -- `transfer` or `cheque`; set by `refund`.
    refund_method TEXT,
    -- The Refund `refund` requested.
    refund_id INTEGER REFERENCES "wl_payments_refunds" (id),
    status TEXT NOT NULL DEFAULT 'held',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_deposits_status" ON "wl_payments_deposits" (status);
CREATE INDEX "wl_payments_deposits_party" ON "wl_payments_deposits" (party_id);
CREATE INDEX "wl_payments_deposits_source" ON "wl_payments_deposits" (source_type, source_id);
