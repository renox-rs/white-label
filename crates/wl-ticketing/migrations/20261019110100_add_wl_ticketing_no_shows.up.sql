-- Vendor no-shows (wl-ticketing §4.1, issue #150, `WRK.RUL.19`): a vendor
-- that did not start a ticket in time, recorded against the vendor and the
-- contract. One row per ticket and scheduled time, so a new visit time with
-- the same vendor is a new occurrence.
CREATE TABLE "wl_ticketing_no_shows" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL REFERENCES "wl_ticketing_tickets" (id) ON DELETE CASCADE,
    vendor_party_id INTEGER NOT NULL,
    -- The contract the ticket is handled under; NULL for an ad hoc vendor.
    contract_id INTEGER,
    -- The visit time (or the response due time) the vendor missed.
    scheduled_at TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (ticket_id, scheduled_at)
);

CREATE INDEX "wl_ticketing_no_shows_contract" ON "wl_ticketing_no_shows" (contract_id);
CREATE INDEX "wl_ticketing_no_shows_vendor" ON "wl_ticketing_no_shows" (vendor_party_id);
