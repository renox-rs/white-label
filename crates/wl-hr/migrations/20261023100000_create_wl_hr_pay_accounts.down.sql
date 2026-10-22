ALTER TABLE "wl_hr_sif_lines" DROP COLUMN account_last4;
ALTER TABLE "wl_hr_sif_lines" DROP COLUMN pay_account_id;
DROP INDEX "wl_hr_pay_accounts_employee";
DROP TABLE "wl_hr_pay_accounts";
