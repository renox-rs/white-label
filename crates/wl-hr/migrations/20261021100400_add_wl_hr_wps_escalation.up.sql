-- `HRM.RUL.11` (wl-hr §6, issue #204): the day the MD and the accountant were
-- told that a rejected or partly rejected run is still open before the WPS due
-- date. A run is escalated once.
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN escalated_on TEXT;
