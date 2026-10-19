DROP INDEX IF EXISTS "wl_parties_screenings_stale";
ALTER TABLE "wl_parties_screenings" DROP COLUMN superseded_by;
ALTER TABLE "wl_parties_screenings" DROP COLUMN rerun_of;
ALTER TABLE "wl_parties_screenings" DROP COLUMN lists_used;
