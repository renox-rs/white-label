-- The absence case of a worker who has not turned up (09 Manpower §4.7 row *The worker
-- absconds*, §7.2 `MPW.RUL.09`, BL-MPW-17): the ticket of category *absence* HR works on, the
-- last day present (day 0), the MoHRE absence report number and date HR recorded, the day of
-- the last daily reminder to HR, when the follow-up after opening was done, and how it closed.
CREATE TABLE "manpower_absence_cases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL,
    deployment_id INTEGER NOT NULL REFERENCES "manpower_deployments" (id),
    -- The wl-ticketing ticket; none when ticketing is off.
    ticket_id INTEGER,
    last_present_on TEXT NOT NULL,
    opened_on TEXT NOT NULL,
    report_number TEXT,
    report_date TEXT,
    reminded_on TEXT,
    announced_at TEXT,
    closed_on TEXT,
    close_reason TEXT,
    closed_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_absence_cases_employee" ON "manpower_absence_cases" (employee_id);
