DROP TABLE IF EXISTS "wl_delivery_traces";
ALTER TABLE "wl_delivery_deliveries" DROP COLUMN reviewed_at;
ALTER TABLE "wl_delivery_deliveries" DROP COLUMN review_note;
ALTER TABLE "wl_delivery_deliveries" DROP COLUMN review_reason;
ALTER TABLE "wl_delivery_deliveries" DROP COLUMN review_ticket_id;
ALTER TABLE "wl_delivery_deliveries" DROP COLUMN frozen;
