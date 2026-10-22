ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN resubmit_note;
ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN rejection_notice;
ALTER TABLE "wl_hr_payroll_runs" DROP COLUMN rejection_reason;
DROP TABLE IF EXISTS "wl_hr_establishments";
DROP TABLE IF EXISTS "wl_hr_sif_lines";
DROP TABLE IF EXISTS "wl_hr_sif_files";
