-- The broadcast a marketing message belongs to (#61 A3). A recipient gets at
-- most one message per broadcast, so a retry after a crash skips the ones
-- already queued. Messages of no broadcast have a NULL, which the index
-- treats as distinct.
ALTER TABLE "wl_notify_messages" ADD COLUMN broadcast_id INTEGER;
CREATE UNIQUE INDEX "wl_notify_messages_broadcast_party"
    ON "wl_notify_messages" (broadcast_id, party_id);
