DROP TABLE IF EXISTS "wl_delivery_hand_over_stops";
DROP TABLE IF EXISTS "wl_delivery_hand_overs";
ALTER TABLE "wl_delivery_runs" DROP COLUMN aborted_at;
ALTER TABLE "wl_delivery_runs" DROP COLUMN aborted_by;
ALTER TABLE "wl_delivery_runs" DROP COLUMN abort_note;
