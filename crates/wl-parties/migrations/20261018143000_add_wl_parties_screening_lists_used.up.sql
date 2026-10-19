-- The lists a screening used and the re-run link (wl-parties §4, issue #40):
-- a run on a stale list is re-run as a new screening after the list updates.
ALTER TABLE "wl_parties_screenings" ADD COLUMN lists_used TEXT NOT NULL DEFAULT '[]';
-- The screening this one re-runs.
ALTER TABLE "wl_parties_screenings" ADD COLUMN rerun_of INTEGER REFERENCES "wl_parties_screenings" (id) ON DELETE SET NULL;
-- The screening that re-ran this one; none while it stands.
ALTER TABLE "wl_parties_screenings" ADD COLUMN superseded_by INTEGER REFERENCES "wl_parties_screenings" (id) ON DELETE SET NULL;
CREATE INDEX "wl_parties_screenings_stale" ON "wl_parties_screenings" (lists_stale, superseded_by);
