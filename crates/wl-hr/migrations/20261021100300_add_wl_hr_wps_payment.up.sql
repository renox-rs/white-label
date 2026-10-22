-- A partly failed WPS file, `regenerate` and `confirm_paid` (wl-hr §4
-- *Payroll run*, §5.3 steps 6 and 7, issue #203).

-- The bank's result or confirmation file of a SIF version (a reference to the
-- attached file): the lines of the version it shows paid are confirmed.
ALTER TABLE "wl_hr_sif_files" ADD COLUMN confirmation TEXT;

-- The payslip of a line is published to the employee portal once; the day it
-- was is kept so that `confirm_paid` does not publish it again.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN payslip_published_at TEXT;

-- A failed line moved out of WPS and paid another approved way: the method
-- (`transfer`, `cheque`, `cash`) and its reference, e.g. `TR-88`.
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN paid_method TEXT;
ALTER TABLE "wl_hr_payroll_lines" ADD COLUMN paid_reference TEXT;
