-- Whether a contact point can still be reached (wl-parties §3, #58): a hard
-- delivery failure marks it `invalid`, editing or verifying it makes it
-- `valid` again. The point is never deleted and keeps its primary flag.
ALTER TABLE "wl_parties_contact_points" ADD COLUMN status TEXT NOT NULL DEFAULT 'valid';
-- When it was marked invalid; NULL while valid.
ALTER TABLE "wl_parties_contact_points" ADD COLUMN invalid_since TEXT;
-- Why: `hard_bounce`, `invalid_number` or `unknown_account`; NULL while valid.
ALTER TABLE "wl_parties_contact_points" ADD COLUMN invalid_reason TEXT;
