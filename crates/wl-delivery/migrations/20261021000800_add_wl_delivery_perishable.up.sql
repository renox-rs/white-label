-- The perishable policy after a failed attempt (wl-delivery §4.1 `reattempt`,
-- `return`, `discard`, `WRK.RUL.16`, #163) and the hand-over window of a
-- perishable stop (§4.2 `hand_over`, #166).
--
-- `placed_at` and `promise_minutes` come from the source (06: the order's
-- placed time and the brand's delivery promise): the window end is the
-- placed time plus the promise. `resolution` is what the rule decided for a
-- failed delivery that waits for its goods: `return` (confirmed at the
-- origin) or `discard` (the driver's photo and reason). The `discard_*` and
-- `wastage_*` columns keep what the driver gave and what the inventory made.
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN placed_at TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN promise_minutes INTEGER;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN resolution TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN discard_reason TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN discard_photo_ref TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN discarded_at TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN wastage_type TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN wastage_id INTEGER;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN wastage_value INTEGER NOT NULL DEFAULT 0;
-- The arrival the dispatcher expects a perishable stop to have on the
-- receiving run.
ALTER TABLE "wl_delivery_hand_over_stops" ADD COLUMN expected_arrival TEXT;
