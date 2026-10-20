-- What `report_absconding` (wl-hr §4 *Employee*, issue #197) reads and
-- writes. The payroll run, the final settlement and the visa case are the
-- minimal base the exit needs: the lines held from a new SIF file, the
-- settlement drafted and the cancellation case opened. Their own lifecycles
-- (BL-HRM-11 to 14, 16 to 18, 22 to 24, the visa cases) extend these tables.

-- The absence cases of the verticals, mirrored (`MPW.RUL.09`, `CON.RUL.08`).
-- `last_present_on` is the last day with attendance or approved leave, from
-- which the case counts; the case is open while `closed_on` is empty.
CREATE TABLE "wl_hr_absence_cases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id) ON DELETE CASCADE,
    -- The case as the vertical names it, e.g. `ticketing.ticket`.
    case_type TEXT NOT NULL,
    case_id INTEGER NOT NULL,
    opened_on TEXT NOT NULL,
    last_present_on TEXT NOT NULL,
    closed_on TEXT,
    close_reason TEXT,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (case_type, case_id)
);

CREATE INDEX "wl_hr_absence_cases_employee" ON "wl_hr_absence_cases" (employee_id, closed_on);

-- Contact attempts during an absence: date, method, by whom, outcome.
CREATE TABLE "wl_hr_contact_attempts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id) ON DELETE CASCADE,
    attempted_on TEXT NOT NULL,
    method TEXT NOT NULL,
    by_user_id INTEGER,
    outcome TEXT NOT NULL DEFAULT 'no_answer',
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_contact_attempts_employee" ON "wl_hr_contact_attempts" (employee_id, attempted_on);

-- The MoHRE absence report on the employee. `access_disabled_at` is when the
-- portal and attendance were switched off with it.
CREATE TABLE "wl_hr_absence_reports" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id) ON DELETE CASCADE,
    report_number TEXT NOT NULL,
    report_date TEXT NOT NULL,
    report_file_id INTEGER,
    reported_on TEXT NOT NULL,
    -- `valid` or `withdrawn`.
    status TEXT NOT NULL DEFAULT 'valid',
    withdrawal_reference TEXT,
    returned_on TEXT,
    access_disabled_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_absence_reports_employee" ON "wl_hr_absence_reports" (employee_id, status);

-- Payroll runs and lines, as far as the exit needs them (wl-hr §4 *Payroll
-- run*): one run per entity and period, with the SIF versions generated so
-- far.
CREATE TABLE "wl_hr_payroll_runs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER,
    -- The month, `2026-11`.
    period TEXT NOT NULL,
    run_type TEXT NOT NULL DEFAULT 'regular',
    status TEXT NOT NULL DEFAULT 'draft',
    -- How many SIF versions were generated.
    sif_version INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_payroll_runs_period" ON "wl_hr_payroll_runs" (period, status);

CREATE TABLE "wl_hr_payroll_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES "wl_hr_payroll_runs" (id) ON DELETE CASCADE,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- AED fils.
    net_amount INTEGER NOT NULL DEFAULT 0,
    -- `included`, `held`, `paid` or `failed`.
    status TEXT NOT NULL DEFAULT 'included',
    held_reason TEXT,
    failure_reason TEXT,
    -- The SIF version that carries the line; empty when it is in none.
    sif_version INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_payroll_lines_run" ON "wl_hr_payroll_lines" (run_id, status);
CREATE INDEX "wl_hr_payroll_lines_employee" ON "wl_hr_payroll_lines" (employee_id, status);

-- Final settlements (wl-hr §4 *Final settlement*): the draft of an exit.
CREATE TABLE "wl_hr_final_settlements" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- Why: `resignation`, `termination`, `absconding`, `onboarding cancelled`, `death`.
    reason TEXT NOT NULL DEFAULT '',
    -- `employee` or `estate`.
    payee TEXT NOT NULL DEFAULT 'employee',
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_final_settlements_employee" ON "wl_hr_final_settlements" (employee_id, status);

-- Visa cases (wl-hr §4 *Visa case*): the cancellation case of an exit.
CREATE TABLE "wl_hr_visa_cases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- `new`, `renewal` or `cancellation`.
    case_type TEXT NOT NULL,
    -- The step template variant of a cancellation: `standard`,
    -- `absconding` or `death`.
    variant TEXT NOT NULL DEFAULT 'standard',
    status TEXT NOT NULL DEFAULT 'open',
    -- The MoHRE absence report an absconding case starts from.
    report_number TEXT,
    report_date TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_visa_cases_employee" ON "wl_hr_visa_cases" (employee_id, case_type, status);
