-- 10 EdTech batch `cancel` and `terminate` (10 section 7.1 *Batch*,
-- BL-EDU-13, BL-EDU-12): what the transition did with each enrolment (written
-- by its effect, read once after the commit), who asked for it, whether the
-- reason is regulatory, and the claim that keeps the after-commit step to one run.
ALTER TABLE "edtech_batches" ADD COLUMN exit_log TEXT NOT NULL DEFAULT '[]';
ALTER TABLE "edtech_batches" ADD COLUMN exit_requested_by INTEGER;
ALTER TABLE "edtech_batches" ADD COLUMN exit_regulatory BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "edtech_batches" ADD COLUMN exit_finished_at TEXT;
