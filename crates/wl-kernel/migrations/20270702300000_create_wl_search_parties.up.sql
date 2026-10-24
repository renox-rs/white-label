-- The parties a search document belongs to (BL-AGT-05): the portal search
-- finds a portal-visible record only for the portal users acting for one of
-- its parties (the invoice's customer, the ticket's requester).
CREATE TABLE "wl_search_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_search_parties_subject_party" ON "wl_search_parties" (subject_type, subject_id, party_id);
CREATE INDEX "wl_search_parties_party" ON "wl_search_parties" (party_id);
