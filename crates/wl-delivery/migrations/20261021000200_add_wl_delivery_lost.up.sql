-- A lost delivery (wl-delivery §4.1 `mark_lost`, `found`, `WRK.RUL.17`,
-- #162): why and when it was marked lost, who bears the loss, and the claim
-- ticket that was opened for it.
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN lost_reason TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN lost_liability TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN lost_at TEXT;
ALTER TABLE "wl_delivery_deliveries" ADD COLUMN claim_ticket_id INTEGER;
