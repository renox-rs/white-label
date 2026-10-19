CREATE TABLE "wl_invoicing_reminder_logs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id) ON DELETE CASCADE,
    -- The rule that sent it: `FIN.RUL.01`.
    rule TEXT NOT NULL,
    -- Days relative to the due date: negative before it, positive after it.
    step INTEGER NOT NULL,
    -- The notification type asked for.
    notification_type TEXT NOT NULL,
    -- The type's first channel.
    channel TEXT NOT NULL,
    -- The first message id the notifier returned; NULL without wl-notify.
    message_id INTEGER,
    sent_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_invoicing_reminder_logs_invoice_step"
    ON "wl_invoicing_reminder_logs" (invoice_id, step);
