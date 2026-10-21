-- 05 Clinics `STK.MOV.01`, `STK.LOT.01` (BL-CLN-13): the lot of every
-- consumable issued to a visit. A consumable line of a lot-tracked item
-- names the lot the desk picked or scanned (the earliest expiry is proposed);
-- when the visit ends the stock is issued from the branch store and each lot
-- it came from, or the pending issue for the short part, is one row.
ALTER TABLE "clinics_charge_lines" ADD COLUMN lot_tracked BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "clinics_charge_lines" ADD COLUMN lot_id INTEGER;
ALTER TABLE "clinics_charge_lines" ADD COLUMN lot_number TEXT;
ALTER TABLE "clinics_charge_lines" ADD COLUMN issued_at TEXT;

CREATE TABLE "clinics_consumable_issues" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    visit_id INTEGER NOT NULL REFERENCES "clinics_visits" (id),
    charge_line_id INTEGER NOT NULL REFERENCES "clinics_charge_lines" (id),
    item_id INTEGER NOT NULL,
    location_id INTEGER NOT NULL,
    lot_id INTEGER,
    lot_number TEXT,
    expiry_date TEXT,
    quantity_milli INTEGER NOT NULL,
    pending_issue_id INTEGER,
    issued_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_consumable_issues_visit_id" ON "clinics_consumable_issues" (visit_id);
CREATE INDEX "clinics_consumable_issues_charge_line_id" ON "clinics_consumable_issues" (charge_line_id);
CREATE INDEX "clinics_consumable_issues_lot_id" ON "clinics_consumable_issues" (lot_id);
