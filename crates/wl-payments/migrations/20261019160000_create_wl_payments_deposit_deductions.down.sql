ALTER TABLE "wl_payments_deposits" DROP COLUMN forfeit_approved_by;
ALTER TABLE "wl_payments_deposits" DROP COLUMN forfeit_reason;
ALTER TABLE "wl_payments_deposits" DROP COLUMN dispute_legal_case;
DROP TABLE IF EXISTS "wl_payments_deposit_disputes";
DROP TABLE IF EXISTS "wl_payments_deposit_deduction_lines";
DROP TABLE IF EXISTS "wl_payments_deposit_deductions";
