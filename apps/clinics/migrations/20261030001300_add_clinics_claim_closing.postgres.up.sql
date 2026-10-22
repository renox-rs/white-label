-- 05 Clinics `CLN.ACL.16` (BL-CLN-09): a claim that missed the payer's
-- first-submission window is closed by `write_off`, its share moved to the
-- patient only when the payer contract allows it (off by default, owner only).
-- The claim keeps how it was closed and that it was *not submitted in time*.
ALTER TABLE "clinics_payer_contracts" ADD COLUMN bill_missed_to_patient BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "clinics_claims" ADD COLUMN close_how TEXT;
ALTER TABLE "clinics_claims" ADD COLUMN close_reason TEXT;
ALTER TABLE "clinics_claims" ADD COLUMN close_amount BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "clinics_claims" ADD COLUMN closed_on DATE;
ALTER TABLE "clinics_claims" ADD COLUMN closed_by BIGINT;
ALTER TABLE "clinics_claims" ADD COLUMN not_submitted_in_time BOOLEAN NOT NULL DEFAULT FALSE;
