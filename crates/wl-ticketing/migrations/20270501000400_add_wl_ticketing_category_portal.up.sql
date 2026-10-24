-- A category open to the portal (BL-TKT-14): offered on the portal's
-- *New request* form. Platform categories are never offered.
ALTER TABLE "wl_ticketing_categories" ADD COLUMN portal INTEGER NOT NULL DEFAULT 0;
