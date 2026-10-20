-- Timesheets and their entries (wl-attendance §3, §4 *Timesheet*; issues #182
-- to #184). The status moves only through the Timesheet machine. The owner is
-- an employee (a crew as owner comes with the crews); an adjustment points to
-- the locked timesheet it corrects (`adjusts_id`), and `adjusted_by` of the
-- original is the list of the rows that point to it.
CREATE TABLE "wl_attendance_timesheets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_employee_id INTEGER NOT NULL,
    period_from TEXT NOT NULL,
    period_to TEXT NOT NULL,
    -- The Subject the hours are for (deployment, project, engagement...).
    context_type TEXT,
    context_id INTEGER,
    -- `regular` or `adjustment`.
    kind TEXT NOT NULL DEFAULT 'regular',
    -- Where the hours come from: `attendance`, `manual` or `generated`.
    hours_source TEXT NOT NULL DEFAULT 'attendance',
    -- `internal` or `external`.
    approver_kind TEXT NOT NULL DEFAULT 'internal',
    approver_role TEXT,
    approver_party INTEGER,
    -- `approved`, `on_behalf` or `deemed`.
    approval_basis TEXT,
    -- The consumers (what used the timesheet first, and later).
    billed_by TEXT,
    paid_by TEXT,
    paid_provisionally_by TEXT,
    exported_at TEXT,
    -- An adjustment: the locked timesheet it corrects, why, and where it flows.
    adjusts_id INTEGER,
    reason TEXT,
    origin TEXT,
    linked_type TEXT,
    linked_id INTEGER,
    affects TEXT,
    created_by INTEGER,
    submitted_by INTEGER,
    submitted_at TEXT,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_timesheets_owner" ON "wl_attendance_timesheets" (owner_employee_id, period_from);
CREATE INDEX "wl_attendance_timesheets_adjusts" ON "wl_attendance_timesheets" (adjusts_id);

-- Hours on one day against one cost object, by category, in minutes (signed
-- in an adjustment).
CREATE TABLE "wl_attendance_timesheet_entries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timesheet_id INTEGER NOT NULL REFERENCES "wl_attendance_timesheets" (id) ON DELETE CASCADE,
    employee_id INTEGER NOT NULL,
    date TEXT NOT NULL,
    cost_object_type TEXT,
    cost_object_id INTEGER,
    normal_minutes INTEGER NOT NULL DEFAULT 0,
    overtime_minutes INTEGER NOT NULL DEFAULT 0,
    night_minutes INTEGER NOT NULL DEFAULT 0,
    rest_day_minutes INTEGER NOT NULL DEFAULT 0,
    public_holiday_minutes INTEGER NOT NULL DEFAULT 0,
    billable INTEGER NOT NULL DEFAULT 1,
    -- `attendance`, `manual` or `generated`.
    source TEXT NOT NULL DEFAULT 'manual',
    -- The attendance record the hours were proposed from.
    attendance_record_id INTEGER REFERENCES "wl_attendance_records" (id),
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_timesheet_entries_sheet" ON "wl_attendance_timesheet_entries" (timesheet_id);
CREATE INDEX "wl_attendance_timesheet_entries_day" ON "wl_attendance_timesheet_entries" (employee_id, date);
CREATE INDEX "wl_attendance_timesheet_entries_record" ON "wl_attendance_timesheet_entries" (attendance_record_id);
