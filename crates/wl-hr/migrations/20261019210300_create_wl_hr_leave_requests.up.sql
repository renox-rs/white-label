-- Leave requests (wl-hr §3 *Leave request*), as far as onboarding needs them
-- (issue #218): the approved days of leave move the due dates of the open
-- onboarding steps. The leave lifecycle (request, approve, reject, balances,
-- calendars) is the base of BL-HRM-01 and writes these rows.
CREATE TABLE "wl_hr_leave_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- `annual`, `sick`, `unpaid`, ...
    leave_type TEXT NOT NULL DEFAULT 'annual',
    from_date TEXT NOT NULL,
    -- The last day of leave, included.
    to_date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_leave_requests_employee" ON "wl_hr_leave_requests" (employee_id, status);
