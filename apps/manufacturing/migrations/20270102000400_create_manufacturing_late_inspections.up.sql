-- PLT.ACL.10 (BL-MFG-14): the decision on a returned lot typed in later keeps
-- its business time. The row is the late entry's record; the decision is applied
-- to the inspection (lot released, NCR raised) once the entry is recorded or approved.
CREATE TABLE "manufacturing_late_inspections" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    inspection_id INTEGER NOT NULL,
    decision TEXT NOT NULL,
    text TEXT,
    plant_id INTEGER,
    occurred_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    late INTEGER NOT NULL DEFAULT 0,
    late_reason TEXT,
    late_entry_id INTEGER,
    entered_by INTEGER NOT NULL,
    applied_at TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_late_inspections_inspection" ON "manufacturing_late_inspections" (inspection_id);
CREATE INDEX "manufacturing_late_inspections_entry" ON "manufacturing_late_inspections" (late_entry_id);
