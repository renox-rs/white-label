-- 05 Clinics `CLN.RUL.03` (BL-CLN-11): the claims still in draft or ready are
-- chased. The contract keeps N, the days after the visit the billing
-- coordinator is told (below the first-submission window); the claim keeps
-- the day each step fired so a rerun skips it.
ALTER TABLE "clinics_payer_contracts" ADD COLUMN reminder_days INTEGER NOT NULL DEFAULT 7;
ALTER TABLE "clinics_claims" ADD COLUMN window_notified_on TEXT;
ALTER TABLE "clinics_claims" ADD COLUMN window_task_on TEXT;
