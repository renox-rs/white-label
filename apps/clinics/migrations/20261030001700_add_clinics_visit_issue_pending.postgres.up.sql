-- 05 Clinics `STK.MOV.01` (BL-CLN-13): the stock issue of a visit that ended
-- but did not go through (the stock refused, or the store or item was not
-- known). The visit says so and why until a retry issues it.
ALTER TABLE "clinics_visits" ADD COLUMN issue_pending_at TIMESTAMPTZ;
ALTER TABLE "clinics_visits" ADD COLUMN issue_pending_reason TEXT;
