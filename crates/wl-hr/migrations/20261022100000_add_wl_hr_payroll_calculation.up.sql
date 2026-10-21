-- The payroll calculation (wl-hr §4 *Payroll run* `calculate`, §5.3 step 2,
-- issue #206): salary structures, the hours and differences a line was
-- calculated from, and the parts of a line.

-- The pay agreed for an employee from a day on.
CREATE TABLE "wl_hr_salary_structures" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    effective_from TEXT NOT NULL,
    -- AED fils for a month.
    basic INTEGER NOT NULL DEFAULT 0,
    housing INTEGER NOT NULL DEFAULT 0,
    transport INTEGER NOT NULL DEFAULT 0,
    other_allowances INTEGER NOT NULL DEFAULT 0,
    -- AED fils an hour; empty derives it from the basic wage.
    hourly_rate INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_hr_salary_structures_effective" ON "wl_hr_salary_structures" (employee_id, effective_from);

-- What a line is made of (AED fils; the net is the fixed wage, plus the
-- variable pay and the positive differences, minus the deductions).
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN variable_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN difference_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN deductions_amount INTEGER NOT NULL DEFAULT 0;
-- The minutes of timesheet hours the line was calculated from, and those of
-- them that are provisional (the timesheet is not approved yet).
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN hours_minutes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN provisional_minutes INTEGER NOT NULL DEFAULT 0;
-- An exception of the line the preparer reviews, e.g. `missing_timesheet`.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN exception TEXT;

-- The timesheet hours a run was calculated from, one row per timesheet and
-- line; `submit` tells the timesheets which of them were paid.
CREATE TABLE "wl_hr_payroll_hours" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES "wl_hr_payroll_runs" (id) ON DELETE CASCADE,
    line_id INTEGER NOT NULL REFERENCES "wl_hr_payroll_lines" (id) ON DELETE CASCADE,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    timesheet_type TEXT NOT NULL,
    timesheet_id INTEGER NOT NULL,
    provisional INTEGER NOT NULL DEFAULT 0,
    minutes INTEGER NOT NULL DEFAULT 0,
    -- The hours as the timesheets gave them (JSON).
    payload TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_payroll_hours_run" ON "wl_hr_payroll_hours" (run_id);

-- Hours an adjustment timesheet adds or takes back, and the differences of a
-- provisional timesheet once it is approved; each enters one regular run.
CREATE TABLE "wl_hr_hour_differences" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- `adjustment` or `reconciliation`.
    source TEXT NOT NULL,
    -- The adjustment timesheet that holds the hours.
    timesheet_type TEXT NOT NULL,
    timesheet_id INTEGER NOT NULL,
    category TEXT NOT NULL,
    -- Signed.
    minutes INTEGER NOT NULL,
    -- The day whose salary structure prices the hours.
    work_date TEXT NOT NULL,
    -- Signed AED fils, as last calculated.
    amount INTEGER NOT NULL DEFAULT 0,
    -- The run that pays or recovers it.
    run_id INTEGER REFERENCES "wl_hr_payroll_runs" (id),
    -- `open` or `converted` (turned into a recovery plan).
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_hr_hour_differences_source" ON "wl_hr_hour_differences" (source, timesheet_type, timesheet_id, category);
CREATE INDEX "wl_hr_hour_differences_employee" ON "wl_hr_hour_differences" (employee_id, status);
CREATE INDEX "wl_hr_hour_differences_run" ON "wl_hr_hour_differences" (run_id);

-- The recovery a negative difference became (or the plan it was turned into).
ALTER TABLE "wl_hr_adjustments" ADD COLUMN difference_id INTEGER REFERENCES "wl_hr_hour_differences" (id);
