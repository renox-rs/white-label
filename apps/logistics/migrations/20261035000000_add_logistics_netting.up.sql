-- Netting open invoices against the COD in the remittance, and the statement
-- published at `pay` (03 Logistics §4.1 row *Client owes invoices*, §6 *COD
-- remittance*, §7.1 `pay`, `LOG.RUL.06`, `LOG.COD.01`, `FIN.PAY.01`;
-- BL-LOG-16).

-- Whether the client's contract allows netting (off unless set).
ALTER TABLE "logistics_client_terms" ADD COLUMN netting_allowed INTEGER NOT NULL DEFAULT 0;

-- What the offsets of open invoices take off the net, and the message the
-- accountant reads when `pay` sent the remittance back to a draft.
ALTER TABLE "logistics_remittances" ADD COLUMN offset_total INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_remittances" ADD COLUMN message TEXT;

-- One open invoice offset against the COD of a draft remittance. The payment
-- is recorded only when the remittance is paid.
CREATE TABLE "logistics_remittance_offsets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    remittance_id INTEGER NOT NULL REFERENCES "logistics_remittances" (id),
    invoice_id INTEGER NOT NULL,
    invoice_number TEXT NOT NULL DEFAULT '',
    due_date TEXT,
    amount INTEGER NOT NULL,
    -- The payment (method *COD netting*) allocated to the invoice at `pay`.
    payment_id INTEGER,
    payment_number TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_remittance_offsets_remittance" ON "logistics_remittance_offsets" (remittance_id);

-- The statement of a remittance as published to the client portal: one row
-- at `pay`, one more at each `repay`; the earlier ones stay as history.
CREATE TABLE "logistics_statements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    remittance_id INTEGER NOT NULL REFERENCES "logistics_remittances" (id),
    version INTEGER NOT NULL,
    payment_reference TEXT NOT NULL DEFAULT '',
    paid_on TEXT NOT NULL,
    net INTEGER NOT NULL DEFAULT 0,
    offset_total INTEGER NOT NULL DEFAULT 0,
    published_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_statements_version" ON "logistics_statements" (remittance_id, version);
