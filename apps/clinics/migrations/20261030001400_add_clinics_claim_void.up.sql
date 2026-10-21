-- 05 Clinics `CLN.ACL.14` (BL-CLN-10): a claim created in error is voided
-- before it was ever sent. It keeps the reason code, the note and, for a
-- duplicate, the claim it duplicates; the claim a re-split makes keeps the
-- one it replaces; a visit left without a payer split is flagged *coverage
-- to resolve*.
ALTER TABLE "clinics_claims" ADD COLUMN void_code TEXT;
ALTER TABLE "clinics_claims" ADD COLUMN void_note TEXT;
ALTER TABLE "clinics_claims" ADD COLUMN void_duplicate_of INTEGER;
ALTER TABLE "clinics_claims" ADD COLUMN voided_on TEXT;
ALTER TABLE "clinics_claims" ADD COLUMN voided_by INTEGER;
ALTER TABLE "clinics_claims" ADD COLUMN replaces_claim_id INTEGER;
ALTER TABLE "clinics_visits" ADD COLUMN coverage_to_resolve BOOLEAN NOT NULL DEFAULT FALSE;
