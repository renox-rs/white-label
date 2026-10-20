-- A unit leaves a management agreement while the agreement goes on (01
-- Property §7.1 *Management agreement* `transfer_unit`, BL-PRP-23): the
-- request with its approval, a unit listed under the new owner's agreement
-- that waits for the transfer, and the legal entity that issues the invoices
-- of a recreated charge.
CREATE TABLE "property_unit_transfers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id INTEGER NOT NULL REFERENCES "property_management_agreements" (id),
    unit_id INTEGER NOT NULL REFERENCES "property_units" (id),
    -- The day the sale completes or the management stops for the unit.
    transfer_date TEXT NOT NULL,
    -- sale (still managed for the new owner) or unmanaged.
    kind TEXT NOT NULL,
    -- The new owner and the agreement that lists the unit from the transfer date.
    new_owner_party_id INTEGER,
    new_agreement_id INTEGER,
    -- requested, approved or rejected.
    status TEXT NOT NULL DEFAULT 'requested',
    requested_by INTEGER,
    requested_on TEXT NOT NULL,
    decided_by INTEGER,
    decided_on TEXT,
    note TEXT,
    -- The new owner's receipt for the deposit that stays held (a sale).
    deposit_receipt TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_unit_transfers_agreement" ON "property_unit_transfers" (agreement_id);
CREATE INDEX "property_unit_transfers_unit" ON "property_unit_transfers" (unit_id);

ALTER TABLE "property_agreement_units" ADD COLUMN awaiting_transfer INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "property_lease_charges" ADD COLUMN legal_entity_id INTEGER;
