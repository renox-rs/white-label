-- PLT.ACL.10 (BL-MFG-14): a production report typed in after the shift from
-- the paper job card keeps its business time. The row is the late entry's
-- record; the report is posted from it once the entry is recorded or approved.
CREATE TABLE "manufacturing_late_reports" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    work_order_id INTEGER NOT NULL,
    plant_id INTEGER NOT NULL,
    good_milli INTEGER NOT NULL DEFAULT 0,
    scrap_milli INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    occurred_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    late INTEGER NOT NULL DEFAULT 0,
    late_reason TEXT,
    late_entry_id INTEGER,
    entered_by INTEGER NOT NULL,
    report_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "manufacturing_late_reports_order" ON "manufacturing_late_reports" (work_order_id);
CREATE INDEX "manufacturing_late_reports_entry" ON "manufacturing_late_reports" (late_entry_id);
