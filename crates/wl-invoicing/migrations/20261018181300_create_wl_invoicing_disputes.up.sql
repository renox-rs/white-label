-- An invoice dispute (#67): flagged by staff or the party, resolved
-- `upheld` or `credited`. Past disputes stay as rows.
CREATE TABLE "wl_invoicing_disputes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES "wl_invoicing_invoices" (id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    -- The disputed amount in fils, fixed when it was raised.
    amount INTEGER NOT NULL,
    -- JSON list of the disputed line positions; empty when not tied to lines.
    lines TEXT NOT NULL DEFAULT '[]',
    -- The `billing_dispute` ticket; NULL when ticketing is off.
    ticket_id INTEGER,
    -- Who raised it: a user or a party (both NULL for the system).
    raised_by_user_id INTEGER,
    raised_by_party_id INTEGER,
    -- `staff`, `portal` or `system`.
    raised_via TEXT NOT NULL,
    raised_at TEXT NOT NULL,
    -- `upheld` or `credited`; NULL while the dispute is open.
    outcome TEXT,
    resolution_note TEXT,
    agreed_amount INTEGER,
    credit_note_id INTEGER,
    approval_request_id INTEGER,
    resolved_by_user_id INTEGER,
    resolved_at TEXT,
    -- The last day chasing stays paused after `upheld`.
    grace_until TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_invoicing_disputes_invoice" ON "wl_invoicing_disputes" (invoice_id);
-- One open dispute per invoice.
CREATE UNIQUE INDEX "wl_invoicing_disputes_open"
    ON "wl_invoicing_disputes" (invoice_id) WHERE outcome IS NULL;

-- The invoice's open dispute, and the end of the chasing grace period.
ALTER TABLE "wl_invoicing_invoices" ADD COLUMN dispute_id INTEGER;
ALTER TABLE "wl_invoicing_invoices" ADD COLUMN dispute_grace_until TEXT;
