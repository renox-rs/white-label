-- The cheque handover receipt (wl-payments §4 Cheque `return`, BL-PAY-30): one
-- row per return call, under the receipt reference the cheques' history rows
-- carry, with the receipt's number and generated document once issued.
CREATE TABLE "wl_payments_cheque_handovers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt TEXT NOT NULL UNIQUE,
    legal_entity_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    receiver_party_id INTEGER NOT NULL,
    cheques TEXT NOT NULL DEFAULT '[]',
    total INTEGER NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'AED',
    reason TEXT NOT NULL DEFAULT '',
    returned_on TEXT NOT NULL,
    number TEXT,
    document_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cheque_handovers_party" ON "wl_payments_cheque_handovers" (party_id);
