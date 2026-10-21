DROP TABLE IF EXISTS "wl_hr_adjustments";
ALTER TABLE "wl_hr_payroll_lines" DROP COLUMN fixed_wage;
ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN approved_by;
ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN prepared_by;
