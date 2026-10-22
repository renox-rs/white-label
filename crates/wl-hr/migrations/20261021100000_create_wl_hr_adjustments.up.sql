-- Payroll preparation and the salary recoveries the Art. 25 caps check
-- (wl-hr §4 *Payroll run*, §6 `HRM.ACL.04` and `HRM.ACL.06`, issue #205).

-- Who prepared (`calculate`) and who approved the run.
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN prepared_by INTEGER;
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN approved_by INTEGER;

-- The line's gross fixed wage for the period (basic plus fixed allowances),
-- AED fils: the basis of the caps.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN fixed_wage INTEGER NOT NULL DEFAULT 0;

-- One-off deductions from salary. `run_id` is the run that took the
-- adjustment into a line; empty while it waits for a run of its period.
CREATE TABLE "wl_hr_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    kind TEXT NOT NULL,
    -- AED fils, always positive.
    amount INTEGER NOT NULL,
    -- The month it is deducted in, `2026-11`.
    period TEXT NOT NULL,
    run_id INTEGER REFERENCES "wl_hr_payroll_runs" (id),
    note TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_adjustments_employee" ON "wl_hr_adjustments" (employee_id, period);
CREATE INDEX "wl_hr_adjustments_run" ON "wl_hr_adjustments" (run_id);
