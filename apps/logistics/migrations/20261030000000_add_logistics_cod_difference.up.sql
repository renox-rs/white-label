-- Who absorbs a COD difference and what is recorded for it (03 Logistics
-- §4.1 *Recipient pays less than the COD amount*, §6, `WRK.ACL.09`,
-- `LOG.COD.01`; BL-LOG-08).
-- The cash collected at the stop, in fils (the difference is the COD less it).
ALTER TABLE "logistics_shipments" ADD COLUMN cod_collected INTEGER NOT NULL DEFAULT 0;
-- The client's instruction to receive less (who, when, how); required to
-- let the client absorb a difference.
ALTER TABLE "logistics_shipments" ADD COLUMN cod_instruction TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN cod_instruction_by INTEGER;
-- The approver who chose the absorber.
ALTER TABLE "logistics_shipments" ADD COLUMN cod_approved_by INTEGER;

-- A COD loss: the difference the business absorbed (the client still gets
-- the full COD). One per shipment.
CREATE TABLE "logistics_cod_losses" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    client_party_id INTEGER NOT NULL,
    -- The depot that bears it; none until the depot is a scope unit.
    depot TEXT,
    amount INTEGER NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    approved_by INTEGER,
    recorded_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_cod_losses_shipment" ON "logistics_cod_losses" (shipment_id);
