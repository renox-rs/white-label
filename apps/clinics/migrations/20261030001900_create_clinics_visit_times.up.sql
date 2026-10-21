-- 05 Clinics, late entry after an outage (BL-CLN-15, #363, PLT.ACL.10): the
-- business time of a check-in, a check-out or a consumable entered later from
-- the paper sheet, with the late-entry flag when it is late. The visit keeps
-- the business time in its own columns; this is the record of how it came.
CREATE TABLE "clinics_visit_times" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    visit_id INTEGER NOT NULL REFERENCES "clinics_visits" (id),
    kind TEXT NOT NULL,
    charge_line_id INTEGER,
    occurred_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    late BOOLEAN NOT NULL DEFAULT FALSE,
    late_reason TEXT,
    late_entry_id INTEGER,
    entered_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_visit_times_visit_id" ON "clinics_visit_times" (visit_id);
