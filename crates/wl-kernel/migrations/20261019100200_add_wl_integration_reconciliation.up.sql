-- The failure run a reconciliation task is about (#30): the time of its first
-- failed call, kept after the connection recovers.
ALTER TABLE "wl_integration_connections" ADD COLUMN outage_since TEXT;

CREATE TABLE "wl_integration_reconciliations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_key TEXT NOT NULL,
    outage_since TEXT NOT NULL,
    lines TEXT NOT NULL DEFAULT '[]',
    ticket_id INTEGER,
    ticket_number TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_integration_reconciliations_outage" ON "wl_integration_reconciliations" (connection_key, outage_since);
