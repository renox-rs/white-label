-- 08 Auto: a job card stopped after the work started (`AUT.ACL.21`, 08 section
-- 7.1 *Job card* `cancel`) and the release of the vehicle. What the advisor
-- recorded (the reason code, the customer's acknowledgement, the value of the
-- work done and what was billed), whether the customer pays on account, and
-- the release of a declined or cancelled job card.
ALTER TABLE "auto_job_cards" ADD COLUMN closed_code TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN stopped_by INTEGER;
ALTER TABLE "auto_job_cards" ADD COLUMN ack_method TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN ack_name TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN ack_signature TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN ack_witness TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN ack_document TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN ack_at TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN work_done_value INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "auto_job_cards" ADD COLUMN billed_net INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "auto_job_cards" ADD COLUMN bill_lowered BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_job_cards" ADD COLUMN on_account BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "auto_job_cards" ADD COLUMN closing_done_at TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN booking_cancelled_at TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN released_at TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN released_to TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN release_condition TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN release_signature TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN released_by INTEGER;

-- A special-order part billed on a stopped job is handed over with the
-- vehicle; the purchase order it was ordered on is kept for the purchaser's
-- task to revise it.
ALTER TABLE "auto_job_part_lines" ADD COLUMN handed_over_at TEXT;
ALTER TABLE "auto_job_part_lines" ADD COLUMN purchase_order_id INTEGER;
