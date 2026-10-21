-- A lost shipment and the goods found again (03 Logistics §4.1 row *Shipment
-- cannot be found*, §4.4 steps 3 and 4, §7.1 rows `mark_lost` and `found`,
-- `WRK.RUL.17`, `LOG.ACL.14`; BL-LOG-11, BL-LOG-12).
ALTER TABLE "logistics_shipments" ADD COLUMN lost_reason TEXT;
ALTER TABLE "logistics_shipments" ADD COLUMN lost_at TEXT;
-- The state it was lost from.
ALTER TABLE "logistics_shipments" ADD COLUMN lost_from TEXT;
-- The depot where the goods were scanned when found.
ALTER TABLE "logistics_shipments" ADD COLUMN found_depot TEXT;
-- The COD it expected when it was lost, in fils; expected again when found.
ALTER TABLE "logistics_shipments" ADD COLUMN lost_cod INTEGER NOT NULL DEFAULT 0;
-- The credit the settled claim gave the client, in fils, and when.
ALTER TABLE "logistics_shipments" ADD COLUMN claim_settled_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_shipments" ADD COLUMN claim_settled_at TEXT;
-- What the client chose when the goods were found after a settled claim:
-- `delivery` or `keep`.
ALTER TABLE "logistics_shipments" ADD COLUMN claim_choice TEXT;
-- The draft invoice that takes the claim credit back.
ALTER TABLE "logistics_shipments" ADD COLUMN claim_invoice_id INTEGER;

-- The daily trace of a shipment that is not on a delivery (`LOG.RUL.12`): one
-- open task per shipment, closed by a scan, a note or any change.
CREATE TABLE "logistics_trace_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    shipment_id INTEGER NOT NULL REFERENCES "logistics_shipments" (id),
    ticket_id INTEGER,
    -- The latest scan, tracking event or note when the task was opened.
    last_seen_at TEXT NOT NULL,
    escalated_at TEXT,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_trace_tasks_shipment" ON "logistics_trace_tasks" (shipment_id);
