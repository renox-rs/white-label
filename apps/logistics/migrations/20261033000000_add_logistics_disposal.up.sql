-- Goods destroyed on the client's written instruction (03 Logistics §4.1 row
-- *Parcel damaged at the hub*, §7.1 row `dispose`, `LOG.ACL.14`; BL-LOG-06).
ALTER TABLE "logistics_shipments" ADD COLUMN disposal_reason TEXT;
-- The references of the photos of the goods, one per line.
ALTER TABLE "logistics_shipments" ADD COLUMN disposal_photos TEXT;
-- The reference of the client's written instruction (e-mail, letter).
ALTER TABLE "logistics_shipments" ADD COLUMN disposal_instruction TEXT;
-- Whether the hub classed the goods as dangerous (no instruction needed).
ALTER TABLE "logistics_shipments" ADD COLUMN disposal_dangerous INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_shipments" ADD COLUMN disposed_at TEXT;
