-- Who approved a timesheet and when, and the proof of an approval given
-- outside the system (wl-attendance §3 *Approval evidence*, §4 `approve`,
-- `approve_on_behalf`; issue #185).
ALTER TABLE "wl_attendance_timesheets" ADD COLUMN approved_by INTEGER;
ALTER TABLE "wl_attendance_timesheets" ADD COLUMN approved_at TEXT;

-- The signed sheet that stands for the external approver's approval.
CREATE TABLE "wl_attendance_approval_evidence" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timesheet_id INTEGER NOT NULL REFERENCES "wl_attendance_timesheets" (id) ON DELETE CASCADE,
    -- The document of wl-documents that holds the signed sheet.
    document_id INTEGER NOT NULL,
    approver_name TEXT NOT NULL,
    signed_on TEXT NOT NULL,
    attached_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_approval_evidence_sheet" ON "wl_attendance_approval_evidence" (timesheet_id);
