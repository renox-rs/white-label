DROP INDEX "wl_hr_payroll_lines_source";
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN late_warning;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN carry_reason;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN hold_cleared_on;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN released_into;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN source_line_id;
