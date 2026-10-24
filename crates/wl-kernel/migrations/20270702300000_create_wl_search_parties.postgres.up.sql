-- The parties a search document belongs to (BL-AGT-05): the portal search
-- finds a portal-visible record only for the portal users acting for one of
-- its parties (the invoice's customer, the ticket's requester).
CREATE TABLE "wl_search_parties" (
    id BIGSERIAL PRIMARY KEY,
    subject_type TEXT NOT NULL,
    subject_id BIGINT NOT NULL,
    party_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX "wl_search_parties_subject_party" ON "wl_search_parties" (subject_type, subject_id, party_id);
CREATE INDEX "wl_search_parties_party" ON "wl_search_parties" (party_id);
