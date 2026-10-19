-- Back to invoice-only tasks; the tasks of supplier bills are dropped.
CREATE TABLE "wl_invoicing_tasks_old" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    invoice_id INTEGER NOT NULL,
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

INSERT INTO "wl_invoicing_tasks_old"
    (id, kind, invoice_id, source_type, source_id, amount, key, message,
     role, status, done_by, done_at, created_at, updated_at)
SELECT id, kind, invoice_id, source_type, source_id, amount, key, message,
     role, status, done_by, done_at, created_at, updated_at
FROM "wl_invoicing_tasks"
WHERE invoice_id IS NOT NULL;

DROP TABLE "wl_invoicing_tasks";
ALTER TABLE "wl_invoicing_tasks_old" RENAME TO "wl_invoicing_tasks";

CREATE UNIQUE INDEX "wl_invoicing_tasks_kind_key"
    ON "wl_invoicing_tasks" (kind, key);
CREATE INDEX "wl_invoicing_tasks_invoice"
    ON "wl_invoicing_tasks" (invoice_id);
