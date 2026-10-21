-- A returned remittance transfer and the control of a change of the client's
-- remittance bank account (03 Logistics §3.3 `LOG.ACL.13`, §4.1 row *Bank
-- returns the remittance transfer*, §7.1 rows `return_payment` and `repay`;
-- BL-LOG-17).

-- The client's confirmation, recorded on the remittance, that the account the
-- transfer was returned from is correct (a temporary bank issue).
ALTER TABLE "logistics_remittances" ADD COLUMN account_confirmed_at TEXT;
ALTER TABLE "logistics_remittances" ADD COLUMN account_confirmed_by INTEGER;
ALTER TABLE "logistics_remittances" ADD COLUMN account_confirmed_note TEXT;

-- Each return keeps its bank advice.
CREATE TABLE "logistics_payment_returns" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    remittance_id INTEGER NOT NULL REFERENCES "logistics_remittances" (id),
    -- The bank's return advice (its reference or document) and the reason.
    advice TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    -- What was returned: the account, the reference and the voucher.
    bank_account TEXT,
    payment_reference TEXT,
    payment_number TEXT,
    returned_by INTEGER,
    returned_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_payment_returns_remittance" ON "logistics_payment_returns" (remittance_id);

-- A change of a client's remittance bank account: active only when a second
-- user approved it and the client confirmed it from its known contact.
CREATE TABLE "logistics_account_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    -- The new account (an IBAN).
    account TEXT NOT NULL,
    requested_by INTEGER NOT NULL,
    requested_at TEXT NOT NULL,
    -- `staff` or `portal`.
    via TEXT NOT NULL DEFAULT 'staff',
    -- The client's primary e-mail on record when the request was made: the
    -- confirmation link goes there, never to an address of the request.
    contact TEXT NOT NULL DEFAULT '',
    approved_by INTEGER,
    approved_at TEXT,
    confirmed_at TEXT,
    -- A change not both approved and confirmed by then lapses.
    expires_at TEXT NOT NULL,
    closed_by INTEGER,
    closed_at TEXT,
    closed_note TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_account_changes_client" ON "logistics_account_changes" (client_party_id);
CREATE INDEX "logistics_account_changes_status" ON "logistics_account_changes" (status);
