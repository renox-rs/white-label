-- An adjustment timesheet of origin `client_dispute` linked to an invoice
-- dispute (wl-attendance §4 *Correcting a locked timesheet*; issue #186):
-- the credit note the dispute was resolved with, the day the dispute ended
-- `upheld` before the adjustment was approved, and the attendance evidence
-- that lets the adjustment affect pay.
ALTER TABLE "wl_attendance_timesheets" ADD COLUMN credit_note_id INTEGER;
ALTER TABLE "wl_attendance_timesheets" ADD COLUMN dispute_upheld_at TEXT;

CREATE TABLE "wl_attendance_adjustment_evidence" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timesheet_id INTEGER NOT NULL REFERENCES "wl_attendance_timesheets" (id) ON DELETE CASCADE,
    -- The Subject that shows the attendance was wrong: an attendance record
    -- or a document.
    evidence_type TEXT NOT NULL,
    evidence_id INTEGER NOT NULL,
    attached_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_adjustment_evidence_sheet" ON "wl_attendance_adjustment_evidence" (timesheet_id);
