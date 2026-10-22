-- The provisional hours a payroll run paid for a timesheet the client had not
-- yet approved (wl-attendance §5.7 points 1 and 2; issue #189): per day and
-- category, in minutes, as paid. The reconciliation compares them with the
-- approved hours (issue #190).
CREATE TABLE "wl_attendance_provisional_hours" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timesheet_id INTEGER NOT NULL REFERENCES "wl_attendance_timesheets" (id) ON DELETE CASCADE,
    -- The payroll run, `hr.payroll_run:5`.
    payroll_run TEXT NOT NULL,
    date TEXT NOT NULL,
    category TEXT NOT NULL,
    minutes INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_provisional_hours_sheet" ON "wl_attendance_provisional_hours" (timesheet_id);
