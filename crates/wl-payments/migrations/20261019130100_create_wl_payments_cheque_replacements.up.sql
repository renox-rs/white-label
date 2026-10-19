-- The replacements of a bounced cheque (wl-payments §4 `replace`,
-- `FIN.ACL.02`, issue #104): the new cheques or payments linked to it, which
-- together must cover its open amount before it can be marked replaced.
CREATE TABLE "wl_payments_cheque_replacements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The bounced cheque that is replaced.
    cheque_id INTEGER NOT NULL REFERENCES "wl_payments_cheques" (id),
    -- `cheque` or `payment`.
    kind TEXT NOT NULL,
    -- The replacement cheque or payment.
    ref_id INTEGER NOT NULL,
    -- What it covers, in fils: a cheque's amount, a payment's unallocated
    -- amount when it was linked.
    amount INTEGER NOT NULL,
    linked_by INTEGER,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (kind, ref_id)
);

CREATE INDEX "wl_payments_cheque_replacements_cheque" ON "wl_payments_cheque_replacements" (cheque_id);

-- Why a replaced cheque is still kept: its replacement bounced, or has not
-- cleared yet.
ALTER TABLE "wl_payments_cheques" ADD COLUMN custody_note TEXT;
