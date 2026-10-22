-- The trace of a recall and its notices (wl-inventory §4 *Recall* `trace`,
-- `send_notices`, `close`, issues #234 and #235): the affected lines (who got
-- the recalled goods), the theoretical flag of consumption, and the columns
-- of the trace, the close and the report.
ALTER TABLE "wl_inventory_stock_movements" ADD COLUMN theoretical INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_pending_issues" ADD COLUMN theoretical INTEGER NOT NULL DEFAULT 0;

ALTER TABLE "wl_inventory_recalls" ADD COLUMN traced_at TEXT;
ALTER TABLE "wl_inventory_recalls" ADD COLUMN trace_runs INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_inventory_recalls" ADD COLUMN closed_at TEXT;
ALTER TABLE "wl_inventory_recalls" ADD COLUMN closed_by INTEGER;
-- The recall report, a PDF document kept as text (like the return note).
ALTER TABLE "wl_inventory_recalls" ADD COLUMN report TEXT;

CREATE TABLE "wl_inventory_recall_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recall_id INTEGER NOT NULL REFERENCES "wl_inventory_recalls" (id) ON DELETE CASCADE,
    -- Names the line once per recall, so a re-run adds only new lines.
    trace_key TEXT NOT NULL,
    -- `confirmed`, `uncertain`, `possible`, `dispatch`, `walk_in`.
    kind TEXT NOT NULL,
    -- The visit, order, job card or dispatch; none for a count of walk-in sales.
    subject_type TEXT,
    subject_id INTEGER,
    -- The party to tell; none when the vertical does not know.
    party_id INTEGER,
    lot_id INTEGER REFERENCES "wl_inventory_lots" (id),
    location_id INTEGER REFERENCES "wl_inventory_locations" (id),
    quantity_milli INTEGER NOT NULL DEFAULT 0,
    -- The sales counted, for `walk_in`.
    sales_count INTEGER NOT NULL DEFAULT 1,
    occurred_on TEXT,
    delivery_status TEXT,
    -- `pending`, `sent`, `waived`.
    notice_status TEXT NOT NULL DEFAULT 'pending',
    template TEXT,
    sent_at TEXT,
    sent_by INTEGER,
    waive_reason TEXT,
    waived_by INTEGER,
    waived_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_recall_lines_key" ON "wl_inventory_recall_lines" (recall_id, trace_key);
CREATE INDEX "wl_inventory_recall_lines_subject" ON "wl_inventory_recall_lines" (subject_type, subject_id);
