-- Chargeable tickets (wl-ticketing §3, §4.1, issue #147): when a category
-- invoices its chargeable tickets, the charge lines of a ticket, and the
-- draft invoice made for it.

-- `resolve` (default) or `close` (confirm, close or the auto-close).
ALTER TABLE "wl_ticketing_categories" ADD COLUMN invoice_on TEXT NOT NULL DEFAULT 'resolve';

-- The draft invoice made for the ticket, once there is one: one per ticket.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN invoice_id INTEGER;

-- The party a chargeable ticket is invoiced to when its requester is a user
-- (a portal user pays through their party); NULL bills the requester's party.
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN bill_party_id INTEGER;

-- What a chargeable ticket bills: both the amount the requester is shown and
-- the lines the draft carries.
CREATE TABLE "wl_ticketing_charge_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL REFERENCES "wl_ticketing_tickets" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    description TEXT NOT NULL,
    -- Thousandths: 1000 is one.
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    -- Fils before VAT.
    unit_price INTEGER NOT NULL,
    vat_category TEXT NOT NULL DEFAULT 'standard',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_charge_lines_ticket" ON "wl_ticketing_charge_lines" (ticket_id);
