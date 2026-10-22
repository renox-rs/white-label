-- Reverts the visa refusal and exit columns.

ALTER TABLE "wl_hr_employees" DROP COLUMN portal_closed_at;
ALTER TABLE "wl_hr_employees" DROP COLUMN exit_route;
ALTER TABLE "wl_hr_employees" DROP COLUMN last_working_day;
DROP INDEX "wl_hr_visa_case_steps_case";
DROP TABLE "wl_hr_visa_case_steps";
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN cancellation_not_needed_on;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN cancellation_not_needed_reason;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN cancellation_not_needed_by;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN deadline;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN rejected_on;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN rejection_reason;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN rejection_reference;
ALTER TABLE "wl_hr_visa_cases" DROP COLUMN current_expiry;
