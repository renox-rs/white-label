-- Closing a run (wl-delivery §4.2 `return_to_depot`, `close`, §5.4, #167):
-- the cash the depot counted, when the run came back and closed, and what
-- the depot accounted for per undelivered delivery and per refused line.
ALTER TABLE "wl_delivery_runs" ADD COLUMN cash_counted_amount INTEGER;
ALTER TABLE "wl_delivery_runs" ADD COLUMN cash_counted_by INTEGER;
ALTER TABLE "wl_delivery_runs" ADD COLUMN cash_counted_at TEXT;
ALTER TABLE "wl_delivery_runs" ADD COLUMN returned_at TEXT;
ALTER TABLE "wl_delivery_runs" ADD COLUMN closed_at TEXT;

-- A refused quantity received back or reported missing at run close.
ALTER TABLE "wl_delivery_lines" ADD COLUMN refused_received_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_delivery_lines" ADD COLUMN refused_missing_milli INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_delivery_lines" ADD COLUMN accounted_at TEXT;
ALTER TABLE "wl_delivery_lines" ADD COLUMN accounted_by INTEGER;

-- What the depot user ticked while the run was back at the depot: one row
-- per undelivered delivery (`delivery_line_id` 0; received = 1000 or missing
-- = 1000) and per refused line (received + missing = the refused quantity).
-- Ticking again replaces the row. The lines get their quantities at `close`.
CREATE TABLE "wl_delivery_receipts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES "wl_delivery_runs" (id) ON DELETE CASCADE,
    delivery_id INTEGER NOT NULL REFERENCES "wl_delivery_deliveries" (id) ON DELETE CASCADE,
    delivery_line_id INTEGER NOT NULL DEFAULT 0,
    received_milli INTEGER NOT NULL DEFAULT 0,
    missing_milli INTEGER NOT NULL DEFAULT 0,
    recorded_by INTEGER,
    recorded_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_delivery_receipts_item"
    ON "wl_delivery_receipts" (run_id, delivery_id, delivery_line_id);
