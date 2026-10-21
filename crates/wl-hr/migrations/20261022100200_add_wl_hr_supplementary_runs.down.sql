DROP TABLE "wl_hr_receivables";
ALTER TABLE "wl_hr_final_settlements" DROP COLUMN recoveries;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN cost_object_id;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN cost_object_type;
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN component;
DROP INDEX "wl_hr_payroll_runs_corrects";
ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN reason;
ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN corrects;
