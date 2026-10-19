-- The lifecycle of a management agreement (01 Property §7.1 *Management
-- agreement*, BL-PRP-22): the notice (its date, the party who gave it and
-- the holder the handover goes to), the day it ended, the handover pack with
-- a line per deposit and per cheque not deposited (each with the new
-- holder's receipt), and the marker "handed over on <date>" of a lease.
ALTER TABLE "property_management_agreements" ADD COLUMN notice_on TEXT;
ALTER TABLE "property_management_agreements" ADD COLUMN notice_party TEXT;
ALTER TABLE "property_management_agreements" ADD COLUMN notice_by INTEGER;
ALTER TABLE "property_management_agreements" ADD COLUMN handover_to_party_id INTEGER;
ALTER TABLE "property_management_agreements" ADD COLUMN ended_on TEXT;

ALTER TABLE "property_leases" ADD COLUMN handed_over_on TEXT;

CREATE TABLE "property_handover_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agreement_id INTEGER NOT NULL REFERENCES "property_management_agreements" (id),
    -- The unit and the lease (version 1's id) the money or the cheque is for.
    unit_id INTEGER NOT NULL DEFAULT 0,
    lease_root_id INTEGER NOT NULL DEFAULT 0,
    -- deposit or cheque, and the id in the deposit book or the cheque register.
    kind TEXT NOT NULL,
    item_ref INTEGER NOT NULL DEFAULT 0,
    description TEXT NOT NULL DEFAULT '',
    amount INTEGER NOT NULL DEFAULT 0,
    -- The reference of the new holder's receipt; empty until it is recorded.
    receipt TEXT,
    receipt_by INTEGER,
    receipt_on TEXT,
    -- Handed over (cheque returned, deposit transferred).
    done INTEGER NOT NULL DEFAULT 0,
    -- The transfer of one unit this line belongs to (BL-PRP-23), if any.
    transfer_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_handover_items_agreement" ON "property_handover_items" (agreement_id);
CREATE INDEX "property_handover_items_unit" ON "property_handover_items" (unit_id);
