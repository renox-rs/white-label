-- The notice of cover not found in time (09 Manpower §4.1, §7.2 `MPW.RUL.13`, BL-MPW-16): when
-- the client and the account manager were told that a cover request is still unfilled. One
-- notice per request: none while it is empty.
ALTER TABLE "manpower_requests" ADD COLUMN cover_notice_sent_at TIMESTAMPTZ;
