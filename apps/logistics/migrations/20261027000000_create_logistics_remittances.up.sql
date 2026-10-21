-- The COD remittance, its shipment lines and the adjustment lines (03
-- Logistics §6 *COD remittance*, §7.1 *COD remittance*, `LOG.RUL.06`,
-- `LOG.ACL.07`, `LOG.ACL.12`; BL-LOG-14, BL-LOG-15). Money is fils.

-- What the remittance needs to know of a delivered shipment: when it was
-- delivered, the COD difference that was recorded and who absorbed it, and
-- whether its COD is marked already remitted (BL-LOG-10).
ALTER TABLE "logistics_shipments" ADD COLUMN delivered_at TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN cod_difference INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_shipments" ADD COLUMN cod_difference_reason TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN cod_absorbed_by TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN cod_already_remitted INTEGER NOT NULL DEFAULT 0;

-- The client's COD fee (a stand-in for its rate card, `FIN.RAT.01`), in
-- basis points of the COD collected, and the bank account remittances go to.
ALTER TABLE "logistics_client_terms" ADD COLUMN cod_fee_bp INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_client_terms" ADD COLUMN bank_account TEXT;

CREATE TABLE "logistics_remittances" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `REM-0007`, also the number of the statement.
    number TEXT NOT NULL,
    client_party_id INTEGER NOT NULL,
    -- The end of the period it was drafted for (Sunday 23:59 Asia/Dubai).
    period_end TEXT NOT NULL,
    cod_total INTEGER NOT NULL DEFAULT 0,
    fee_total INTEGER NOT NULL DEFAULT 0,
    adjustment_total INTEGER NOT NULL DEFAULT 0,
    -- Never below 0.
    net INTEGER NOT NULL DEFAULT 0,
    -- What a negative net was set to 0 for, carried to the next draft.
    carried_forward INTEGER NOT NULL DEFAULT 0,
    prepared_by INTEGER,
    approved_by INTEGER,
    approved_at TEXT,
    paid_by INTEGER,
    paid_at TEXT,
    payment_reference TEXT,
    payment_id INTEGER,
    payment_number TEXT,
    bank_account TEXT,
    disputed INTEGER NOT NULL DEFAULT 0,
    dispute_ticket_id INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_remittances_number" ON "logistics_remittances" (number);
CREATE INDEX "logistics_remittances_client" ON "logistics_remittances" (client_party_id);
CREATE INDEX "logistics_remittances_status" ON "logistics_remittances" (status);

CREATE TABLE "logistics_remittance_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    remittance_id INTEGER NOT NULL REFERENCES "logistics_remittances" (id),
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    -- The COD recorded on the shipment.
    cod_amount INTEGER NOT NULL DEFAULT 0,
    -- The difference the client agreed to absorb, with its reason.
    absorbed INTEGER NOT NULL DEFAULT 0,
    absorbed_reason TEXT NOT NULL DEFAULT '',
    -- The client's COD fee on what is paid out for the shipment.
    fee INTEGER NOT NULL DEFAULT 0,
    -- COD less the absorbed difference less the fee.
    amount INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

-- One shipment is on one remittance.
CREATE UNIQUE INDEX "logistics_remittance_lines_shipment" ON "logistics_remittance_lines" (shipment_id);
CREATE INDEX "logistics_remittance_lines_remittance" ON "logistics_remittance_lines" (remittance_id);

CREATE TABLE "logistics_adjustment_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    -- The draft carrying it; none while it waits for the next weekly run.
    remittance_id INTEGER REFERENCES "logistics_remittances" (id),
    -- Positive: owed to the client. Negative: taken back.
    amount INTEGER NOT NULL,
    reason TEXT NOT NULL,
    -- `manual`, `delivery_reversal`, `statement_dispute`, `cod_difference`
    -- or `carried_forward`.
    source TEXT NOT NULL DEFAULT 'manual',
    -- The shipment or the remittance it corrects.
    shipment_id INTEGER,
    corrects_remittance_id INTEGER,
    -- The remittance whose negative net this line carries forward.
    carried_from_id INTEGER,
    prepared_by INTEGER,
    decided_by INTEGER,
    decided_at TEXT,
    decision_note TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_adjustment_lines_client" ON "logistics_adjustment_lines" (client_party_id);
CREATE INDEX "logistics_adjustment_lines_remittance" ON "logistics_adjustment_lines" (remittance_id);
