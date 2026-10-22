-- Correcting a paid run with a supplementary run (wl-hr §3 *Payroll run*:
-- corrects; §4 *Run types*; §5.6 step 1; issue #207).

-- The paid run a supplementary run corrects (always the original run), and
-- why it was made.
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN corrects INTEGER;
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN reason TEXT;
CREATE INDEX "wl_hr_payroll_runs_corrects" ON "wl_hr_payroll_runs" (corrects);

-- On a line of a supplementary run: the component the difference is in
-- (overtime, allowance, ...). On any line: the cost object its cost is
-- posted to (`FIN.CST.01`).
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN component TEXT;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN cost_object_type TEXT;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN cost_object_id INTEGER;

-- The recoveries added to a final settlement while it is not paid (the
-- negative difference of a supplementary run for an employee who left).
ALTER TABLE "wl_hr_final_settlements" ADD COLUMN recoveries INTEGER NOT NULL DEFAULT 0;

-- Money an employee owes after their settlement was paid or without one (a
-- negative difference of a supplementary run, a negative net): a receivable on
-- the employee's party.
CREATE TABLE "wl_hr_receivables" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    party_id INTEGER NOT NULL,
    -- AED fils, positive.
    amount INTEGER NOT NULL,
    source_run_id INTEGER REFERENCES "wl_hr_payroll_runs" (id),
    source_line_id INTEGER REFERENCES "wl_hr_payroll_lines" (id),
    -- `open` or `settled`.
    status TEXT NOT NULL DEFAULT 'open',
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_receivables_employee" ON "wl_hr_receivables" (employee_id, status);
