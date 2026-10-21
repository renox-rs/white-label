-- A dispute flagged because a delivery went under review (wl-invoicing §5.10
-- step 5, #69): the dispute remembers the record that caused it, and the
-- review itself is kept so that a draft invoice billing the delivery is not
-- issued automatically while the review is open.
ALTER TABLE "wl_invoicing_disputes" ADD COLUMN source_type TEXT;
ALTER TABLE "wl_invoicing_disputes" ADD COLUMN source_id INTEGER;
CREATE INDEX "wl_invoicing_disputes_source" ON "wl_invoicing_disputes" (source_type, source_id);

CREATE TABLE "wl_invoicing_delivery_reviews" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The delivery, as `delivery.delivery:42`.
    delivery_type TEXT NOT NULL,
    delivery_id INTEGER NOT NULL,
    -- The delivery's ticket; NULL when ticketing is off.
    ticket_id INTEGER,
    -- JSON list of the source lines the delivery carries (`type:id`).
    origins TEXT NOT NULL DEFAULT '[]',
    -- Whether the review is still open (a reversal closes it, a loss keeps it).
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_delivery_reviews_delivery"
    ON "wl_invoicing_delivery_reviews" (delivery_type, delivery_id);
