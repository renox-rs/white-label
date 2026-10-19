-- Follow-up tasks may point at a supplier bill as well as an invoice
-- (#75, part 4). SQLite cannot drop NOT NULL, so the table is rebuilt:
-- invoice_id becomes optional and bill_id is new. Still unique per
-- (kind, key).
CREATE TABLE "wl_invoicing_tasks_new" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `reversal_after_write_off`, `reversal_refused` or
    -- `bill_reversal_refused`.
    kind TEXT NOT NULL,
    -- No foreign key: a refusal for an unknown record may be logged too.
    invoice_id INTEGER,
    bill_id INTEGER,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    key TEXT NOT NULL,
    message TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL,
    done_by INTEGER,
    done_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

INSERT INTO "wl_invoicing_tasks_new"
    (id, kind, invoice_id, bill_id, source_type, source_id, amount, key,
     message, role, status, done_by, done_at, created_at, updated_at)
SELECT id, kind, invoice_id, NULL, source_type, source_id, amount, key,
     message, role, status, done_by, done_at, created_at, updated_at
FROM "wl_invoicing_tasks";

DROP TABLE "wl_invoicing_tasks";
ALTER TABLE "wl_invoicing_tasks_new" RENAME TO "wl_invoicing_tasks";

CREATE UNIQUE INDEX "wl_invoicing_tasks_kind_key"
    ON "wl_invoicing_tasks" (kind, key);
CREATE INDEX "wl_invoicing_tasks_invoice"
    ON "wl_invoicing_tasks" (invoice_id);
CREATE INDEX "wl_invoicing_tasks_bill"
    ON "wl_invoicing_tasks" (bill_id);
