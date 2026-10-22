-- 05 Clinics claim lifecycle (`CLN.CLM.01`): the submission windows of a payer
-- contract. The first-submission window counts from the visit date, the
-- resubmission window from the remittance that denied the claim; the payer
-- accepts at most `resubmission_limit` resubmissions. A payer that accepts
-- late claims gets a claim after the window with a late reason.
ALTER TABLE "clinics_payer_contracts" ADD COLUMN first_submission_days INTEGER NOT NULL DEFAULT 30;
ALTER TABLE "clinics_payer_contracts" ADD COLUMN resubmission_days INTEGER NOT NULL DEFAULT 30;
ALTER TABLE "clinics_payer_contracts" ADD COLUMN resubmission_limit INTEGER NOT NULL DEFAULT 2;
ALTER TABLE "clinics_payer_contracts" ADD COLUMN accepts_late_claims BOOLEAN NOT NULL DEFAULT FALSE;
