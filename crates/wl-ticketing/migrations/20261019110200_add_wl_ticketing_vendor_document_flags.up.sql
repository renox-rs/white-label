-- The vendor document flag (wl-ticketing §3, §4.6, issue #152): set on a
-- service contract, and on its open planned visits, while a vendor document
-- its categories need is expired or missing (`DocumentExpired`); cleared when
-- a renewed document is accepted, and on a visit also when it is reassigned.

ALTER TABLE "wl_ticketing_service_contracts" ADD COLUMN vendor_document_flag INTEGER NOT NULL DEFAULT 0;
-- The key of the document kind that expired, so that only its renewal clears it.
ALTER TABLE "wl_ticketing_service_contracts" ADD COLUMN vendor_document_kind TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN vendor_document_flag INTEGER NOT NULL DEFAULT 0;
