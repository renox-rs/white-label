-- The monthly invoice (03 Logistics §7.2 `LOG.RUL.07`; BL-LOG-20): when the
-- shipment reached `returned` (the month its return charge is billed in), and
-- one row per client and month that keeps the draft invoice, so that running
-- the rule twice leaves one draft.
ALTER TABLE "logistics_shipments" ADD COLUMN returned_at TEXT;

CREATE TABLE "logistics_billing_runs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    -- The month billed, `2026-10`.
    period TEXT NOT NULL,
    invoice_id INTEGER,
    -- The invoice lines and their net total, in fils.
    line_count INTEGER NOT NULL DEFAULT 0,
    net_total INTEGER NOT NULL DEFAULT 0,
    -- Return charge lines billed at AED 0 (no return price for the client).
    unpriced_returns INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_billing_runs_period" ON "logistics_billing_runs" (client_party_id, period);
