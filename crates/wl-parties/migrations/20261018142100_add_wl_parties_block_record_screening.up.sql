-- The screening a Block record came from (wl-parties §3, `confirm_match`):
-- NULL for a record made by a user's `block`. Lets the records of one
-- screening be found, and keeps a rerun from adding a second one.
ALTER TABLE "wl_parties_block_records"
    ADD COLUMN screening_id INTEGER;

CREATE INDEX "wl_parties_block_records_screening"
    ON "wl_parties_block_records" (screening_id, party_id);
