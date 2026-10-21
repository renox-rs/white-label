-- Releasing held lines (wl-hr §3 *Payroll line*: hold, released into; §5.6
-- step 2; `HRM.RUL.12`, issue #208).

-- The held line that a line of an off-cycle run, or one carried into the
-- next regular run, pays.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN source_line_id INTEGER REFERENCES "wl_hr_payroll_lines" (id);
-- On a held line: the run that finally pays it. Set once.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN released_into INTEGER REFERENCES "wl_hr_payroll_runs" (id);
-- On a held line: the day its hold reason cleared.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN hold_cleared_on TEXT;
-- On a carried line: why it was carried into a regular run, and the warning
-- that it pays after the WPS due date of its period.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN carry_reason TEXT;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN late_warning TEXT;

CREATE INDEX "wl_hr_payroll_lines_source" ON "wl_hr_payroll_lines" (source_line_id);
