-- The recipient rule of a message sent to `contact` (#57): the party whose
-- contacts are meant, the relation (`contact_of` or `guardian_of`) and the
-- position (e.g. `finance`). The chosen person is in `party_id`.
ALTER TABLE "wl_notify_messages" ADD COLUMN contact_of_party_id INTEGER;
ALTER TABLE "wl_notify_messages" ADD COLUMN contact_relation TEXT;
ALTER TABLE "wl_notify_messages" ADD COLUMN contact_position TEXT;
-- The first message of the chain this one is the alternative recipient of.
ALTER TABLE "wl_notify_messages" ADD COLUMN alternative_of INTEGER;
