-- Holding a shipment for an exception and releasing it, and the audit of
-- address corrections (03 Logistics §4.1 *Wrong or incomplete address* and
-- *Parcel damaged at the hub*, §7.1 rows `hold`, `release`, `return`;
-- BL-LOG-05).
ALTER TABLE "logistics_shipments" ADD COLUMN hold_reason TEXT;
-- The state the shipment was in when it was held.
ALTER TABLE "logistics_shipments" ADD COLUMN held_from TEXT;
-- The references of the photos of a damaged parcel, one per line.
ALTER TABLE "logistics_shipments" ADD COLUMN hold_photos TEXT;
-- The client's instruction recorded for the hold (deliver as is, return).
ALTER TABLE "logistics_shipments" ADD COLUMN hold_instruction TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN hold_instruction_by INTEGER;
ALTER TABLE "logistics_shipments" ADD COLUMN hold_instruction_at TEXT;
-- The address confirmed for an *address issue* hold.
ALTER TABLE "logistics_shipments" ADD COLUMN address_confirmed_at TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN address_confirmed_by INTEGER;
ALTER TABLE "logistics_shipments" ADD COLUMN company_liable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_shipments" ADD COLUMN claim_ticket_id INTEGER;
ALTER TABLE "logistics_shipments" ADD COLUMN claim_ticket_number TEXT;

CREATE TABLE "logistics_address_changes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    old_address TEXT NOT NULL DEFAULT '',
    old_area TEXT NOT NULL DEFAULT '',
    new_address TEXT NOT NULL DEFAULT '',
    new_area TEXT NOT NULL DEFAULT '',
    -- The price on the weight received before and after (a zone change
    -- reprices the shipment at once).
    old_price INTEGER NOT NULL DEFAULT 0,
    new_price INTEGER NOT NULL DEFAULT 0,
    changed_by INTEGER,
    changed_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_address_changes_shipment" ON "logistics_address_changes" (shipment_id);
