-- 10 EdTech session flag (10 section 7.2 `EDU.RUL.12`, BL-EDU-10): a session
-- whose tutor will not be eligible or available shows "tutor not available or
-- not eligible" until it is resolved (the tutor changed, the session moved to
-- an eligible date or cancelled, the document renewed). `flag_alert_id` is the
-- alert (tutor and trigger) that raised it; `flag_escalated_at` is when the
-- centre manager was told that it was still open 24 hours before the start.
ALTER TABLE "edtech_sessions" ADD COLUMN flag_reason TEXT;
ALTER TABLE "edtech_sessions" ADD COLUMN flagged_at TIMESTAMPTZ;
ALTER TABLE "edtech_sessions" ADD COLUMN flag_escalated_at TIMESTAMPTZ;
ALTER TABLE "edtech_sessions" ADD COLUMN flag_alert_id BIGINT;
