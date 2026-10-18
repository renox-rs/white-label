-- Follow-up tasks for the accountant when a reversed payment cannot reopen
-- its invoice (#70, part 3). A small record of the module's own, because
-- ticketing may be off. One task per kind and unallocation key, so a
-- repeated PaymentReversed opens none twice.
CREATE TABLE "wl_invoicing_tasks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `reversal_after_write_off` or `reversal_refused`.
    kind TEXT NOT NULL,
    -- No foreign key: a refusal for an unknown invoice may be logged too.
    invoice_id INTEGER NOT NULL,
    -- The money's source, e.g. `payments.payment:3`.
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    -- The idempotency key of the refused unallocation.
    key TEXT NOT NULL,
    message TEXT NOT NULL,
    -- The role that handles it (`accountant`).
    role TEXT NOT NULL,
    -- `open` or `done`.
    status TEXT NOT NULL,
    done_by INTEGER,
    done_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_tasks_kind_key"
    ON "wl_invoicing_tasks" (kind, key);
CREATE INDEX "wl_invoicing_tasks_invoice"
    ON "wl_invoicing_tasks" (invoice_id);
