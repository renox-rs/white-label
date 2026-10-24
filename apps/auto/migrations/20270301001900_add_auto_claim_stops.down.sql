ALTER TABLE "auto_job_part_lines" DROP COLUMN payer;
ALTER TABLE "auto_job_labour_lines" DROP COLUMN payer;
ALTER TABLE "auto_job_cards" DROP COLUMN release_instruction;
ALTER TABLE "auto_job_cards" DROP COLUMN private_estimate;
ALTER TABLE "auto_insurance_claims" DROP COLUMN letter_sent_at;
ALTER TABLE "auto_insurance_claims" DROP COLUMN unlinked_job_card_id;
ALTER TABLE "auto_insurance_claims" DROP COLUMN withdrawal_choice;
