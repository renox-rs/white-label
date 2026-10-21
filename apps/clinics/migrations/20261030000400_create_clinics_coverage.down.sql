ALTER TABLE "clinics_visits" DROP COLUMN consent_id;
ALTER TABLE "clinics_visits" DROP COLUMN coverage_policy_id;
ALTER TABLE "clinics_visits" DROP COLUMN coverage_outcome;
ALTER TABLE "clinics_visits" DROP COLUMN eligibility_positive;
ALTER TABLE "clinics_visits" DROP COLUMN eligibility_check_id;
DROP INDEX IF EXISTS "clinics_self_pay_consents_booking_id";
DROP TABLE IF EXISTS "clinics_self_pay_consents";
DROP INDEX IF EXISTS "clinics_pending_check_ins_booking_id";
DROP TABLE IF EXISTS "clinics_pending_check_ins";
