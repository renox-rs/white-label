-- `return` of a payroll run (wl-hr §4 *Payroll run*, issue #201): the
-- approver's comment on a line to fix, and the reason the run was returned.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN approver_comment TEXT;
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN return_reason TEXT;
