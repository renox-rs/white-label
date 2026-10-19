-- Enquiries (01 Property `PRP.LEA.01`, BL-PRP-27): a prospect's first contact,
-- with the lead source and the campaign (`SAL.CRM.02`) that reach the
-- tenant's party when the enquiry converts.
CREATE TABLE "property_enquiries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prospect_name TEXT NOT NULL,
    contact TEXT NOT NULL DEFAULT '',
    unit_id INTEGER REFERENCES "property_units" (id),
    source TEXT NOT NULL,
    campaign TEXT,
    notes TEXT NOT NULL DEFAULT '',
    recorded_by INTEGER,
    received_on TEXT NOT NULL,
    converted_party_id INTEGER,
    converted_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_enquiries_source" ON "property_enquiries" (source);
CREATE INDEX "property_enquiries_converted" ON "property_enquiries" (converted_party_id);
