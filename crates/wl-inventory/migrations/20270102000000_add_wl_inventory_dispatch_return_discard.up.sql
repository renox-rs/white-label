-- A dispatch that came back (`return`, issue for the returned delivery) or
-- was discarded by wl-delivery (`discard`, issue #242): why, the complaint
-- ticket the return opened, and the photo and the value of the wastage.
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN return_reason TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN complaint_ticket_id INTEGER;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN discard_reason TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN discard_photo TEXT;
ALTER TABLE "wl_inventory_dispatches" ADD COLUMN discard_fils INTEGER NOT NULL DEFAULT 0;
