ALTER TABLE "clinics_visits" DROP COLUMN coverage_to_resolve;
ALTER TABLE "clinics_claims" DROP COLUMN replaces_claim_id;
ALTER TABLE "clinics_claims" DROP COLUMN voided_by;
ALTER TABLE "clinics_claims" DROP COLUMN voided_on;
ALTER TABLE "clinics_claims" DROP COLUMN void_duplicate_of;
ALTER TABLE "clinics_claims" DROP COLUMN void_note;
ALTER TABLE "clinics_claims" DROP COLUMN void_code;
