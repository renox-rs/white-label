-- Refunding a completed order when its delivery is reversed (06 F&B §4.2
-- row *Customer says the order never arrived*, §7.1 *Order*; BL-FNB-07): the
-- sales return of kind *no goods* the reversal raised, how it was resolved
-- (refund, or credit note only for cash on delivery nobody handed over), its
-- amount and the responsible party. The order stays `completed`.
ALTER TABLE "fnb_orders" ADD COLUMN reversal_return_id BIGINT;
ALTER TABLE "fnb_orders" ADD COLUMN reversed_at TIMESTAMPTZ;
ALTER TABLE "fnb_orders" ADD COLUMN reversal_resolution TEXT;
ALTER TABLE "fnb_orders" ADD COLUMN reversal_amount BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "fnb_orders" ADD COLUMN reversal_party TEXT;
