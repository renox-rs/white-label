-- 10 EdTech `transfer` (`EDU.ACL.17`, 10 section 7.1 *Enrolment* `transfer`):
-- the enrolment a student moves out of points to the new one, and the new one
-- to the old, with the value that moved (`carried_paid`, fils): what the
-- student paid for the sessions not yet used, which counts as paid there.
ALTER TABLE "edtech_enrolments" ADD COLUMN carried_paid BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "edtech_enrolments" ADD COLUMN transferred_from_id BIGINT;
ALTER TABLE "edtech_enrolments" ADD COLUMN transferred_to_id BIGINT;
