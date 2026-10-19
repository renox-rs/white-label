-- Employees (wl-hr §3, §4; issue #216): the minimal record that onboarding
-- needs (person, employing entity, job role, scopes, cost rate). Payroll,
-- visas, leave and settlements come with their own issues. The status moves
-- only through the Employee machine.
CREATE TABLE "wl_hr_employees" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The person (a wl-parties party).
    party_id INTEGER NOT NULL,
    -- The staff or portal account, once there is one.
    user_id INTEGER,
    legal_entity_id INTEGER,
    employee_number TEXT NOT NULL DEFAULT '',
    job_title TEXT NOT NULL DEFAULT '',
    -- The job role or trade key, e.g. `driver`.
    job_role TEXT NOT NULL DEFAULT '',
    department TEXT,
    employment_type TEXT NOT NULL DEFAULT 'full_time',
    hire_date TEXT NOT NULL,
    -- AED fils per hour, when set.
    cost_rate_per_hour INTEGER,
    status TEXT NOT NULL DEFAULT 'onboarding',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_employees_party" ON "wl_hr_employees" (party_id);
CREATE INDEX "wl_hr_employees_role" ON "wl_hr_employees" (job_role, status);

-- The scopes (branches) an employee works in; a Subject each.
CREATE TABLE "wl_hr_employee_scopes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id) ON DELETE CASCADE,
    scope_type TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (employee_id, scope_type, scope_id)
);
