-- 08 Auto: a claim that ends before the repair is paid (BL-AUT-08 .. BL-AUT-10).
-- A withdrawn claim keeps the customer's choice (continue privately or stop)
-- and the time its letter to the insurer was queued; a voided claim keeps the
-- job card it was unlinked from. The job card keeps a flag while the private
-- estimate sent after a withdrawal is not signed (`AUT.RUL.17`) and the
-- insurer's instruction a vehicle was released on after a total loss. Every
-- estimate line says who pays it: the customer, or the insurer for the lines
-- it approved.
ALTER TABLE "auto_insurance_claims" ADD COLUMN withdrawal_choice TEXT;
ALTER TABLE "auto_insurance_claims" ADD COLUMN unlinked_job_card_id BIGINT;
ALTER TABLE "auto_insurance_claims" ADD COLUMN letter_sent_at TIMESTAMPTZ;
ALTER TABLE "auto_job_cards" ADD COLUMN private_estimate BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_job_cards" ADD COLUMN release_instruction TEXT;
ALTER TABLE "auto_job_labour_lines" ADD COLUMN payer TEXT NOT NULL DEFAULT 'customer';
ALTER TABLE "auto_job_part_lines" ADD COLUMN payer TEXT NOT NULL DEFAULT 'customer';
