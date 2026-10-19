-- The frequency cap counts a party's messages sent in the last 7 days (#62 A8).
CREATE INDEX "wl_notify_messages_party_sent"
    ON "wl_notify_messages" (party_id, sent_at);
