ALTER TABLE "wl_payments_followup_tasks" DROP COLUMN reminded_on;
ALTER TABLE "wl_payments_followup_tasks" DROP COLUMN due_on;
DROP INDEX IF EXISTS "wl_payments_payments_dispute_reference";
ALTER TABLE "wl_payments_payments" DROP COLUMN dispute_outcome;
ALTER TABLE "wl_payments_payments" DROP COLUMN dispute_evidence_submitted_on;
ALTER TABLE "wl_payments_payments" DROP COLUMN dispute_fee_payment_id;
ALTER TABLE "wl_payments_payments" DROP COLUMN dispute_fee;
ALTER TABLE "wl_payments_payments" DROP COLUMN dispute_opened_on;
