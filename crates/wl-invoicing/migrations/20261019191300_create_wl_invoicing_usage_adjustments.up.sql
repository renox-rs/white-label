-- What an approved adjustment timesheet did to one usage record (§5.2
-- step 5). One row per adjustment and record, so an event received twice
-- changes nothing the second time.
CREATE TABLE "wl_invoicing_usage_adjustments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The adjustment timesheet, `attendance.timesheet:7`.
    adjustment_type TEXT NOT NULL,
    adjustment_id INTEGER NOT NULL,
    -- The timesheet it corrects.
    adjusts_type TEXT NOT NULL,
    adjusts_id INTEGER NOT NULL,
    usage_record_id INTEGER NOT NULL,
    -- `client_dispute`, `attendance_error`...
    origin TEXT,
    -- The signed change in thousandths of an hour.
    delta_milli INTEGER NOT NULL,
    -- `pending`, `quantity` (not yet invoiced), `draft_line`, `credit_note`,
    -- `extra_line`, `extra_invoice` or `refused`.
    outcome TEXT NOT NULL,
    -- The invoice the original record was billed on.
    invoice_id INTEGER,
    credit_note_id INTEGER,
    extra_record_id INTEGER,
    extra_invoice_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_usage_adjustments_once"
    ON "wl_invoicing_usage_adjustments" (adjustment_type, adjustment_id, usage_record_id);
CREATE INDEX "wl_invoicing_usage_adjustments_invoice"
    ON "wl_invoicing_usage_adjustments" (invoice_id);
