-- Who was told at each escalation step of an open dispute (FIN.RUL.15,
-- #68). One row per dispute, step and user, so a retried step never tells
-- anyone twice.
CREATE TABLE "wl_invoicing_dispute_escalations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dispute_id INTEGER NOT NULL REFERENCES "wl_invoicing_disputes" (id) ON DELETE CASCADE,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id) ON DELETE CASCADE,
    step INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    -- `owner`, `receivables` or `finance_manager`.
    role TEXT NOT NULL,
    -- The first message id the notifier returned; NULL without wl-notify.
    message_id INTEGER,
    sent_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_dispute_escalations_user"
    ON "wl_invoicing_dispute_escalations" (dispute_id, step, user_id);
CREATE INDEX "wl_invoicing_dispute_escalations_invoice"
    ON "wl_invoicing_dispute_escalations" (invoice_id);
