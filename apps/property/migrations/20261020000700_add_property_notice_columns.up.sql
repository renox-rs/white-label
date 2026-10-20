-- Notice to end a lease (01 Property §7.1 `record_notice`, BL-PRP-09): the
-- landlord's ground as text, the day a landlord's notice was sent and the
-- day its delivery was proved, and the move-out inspection (its completion,
-- the keys and whether it found defects). The charge types of the client's
-- catalogue the vertical bills (the early termination fee).
ALTER TABLE "property_leases" ADD COLUMN notice_ground TEXT;
ALTER TABLE "property_leases" ADD COLUMN notice_sent_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN notice_delivered_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN inspection_done_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN keys_returned_on TEXT;
ALTER TABLE "property_leases" ADD COLUMN inspection_defects INTEGER NOT NULL DEFAULT 0;

CREATE TABLE "property_charge_types" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL DEFAULT '',
    name_ar TEXT,
    vat_category TEXT NOT NULL DEFAULT 'standard',
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);
