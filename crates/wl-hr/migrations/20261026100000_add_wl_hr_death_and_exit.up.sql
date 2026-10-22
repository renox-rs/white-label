-- Death in service and the exit of an absconded or deceased employee
-- (wl-hr §3 *Employee*, §4 `record_death` and `exit`, issues #199 and #200).

-- The date of death, the day the employee exited and the authority's
-- reference of the cancellation of the residence visa (when no cancellation
-- case was completed).
ALTER TABLE "wl_hr_employees" ADD COLUMN date_of_death TEXT;
ALTER TABLE "wl_hr_employees" ADD COLUMN exited_on TEXT;
ALTER TABLE "wl_hr_employees" ADD COLUMN cancellation_reference TEXT;

-- A line held for a death: the final settlement that took the salary it
-- owes up to the date of death ("settled in the final settlement"), and
-- the amount that was moved. Such a line is never released.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN settled_in_settlement INTEGER;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN settled_amount INTEGER NOT NULL DEFAULT 0;
