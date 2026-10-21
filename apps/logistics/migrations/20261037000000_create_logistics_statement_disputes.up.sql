-- A client's dispute of a COD statement (03 Logistics §4.1 row *Client
-- disputes a COD statement*, §6 *COD remittance*, §7.1 rows `dispute` and
-- `resolve_dispute`, `LOG.ACL.12`; BL-LOG-18). The `disputed` flag and the
-- ticket are on the remittance; this keeps each dispute and its outcome.
CREATE TABLE "logistics_statement_disputes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    remittance_id INTEGER NOT NULL REFERENCES "logistics_remittances" (id),
    reason TEXT NOT NULL,
    raised_by INTEGER NOT NULL,
    raised_at TEXT NOT NULL,
    -- `staff` (customer service on the client's call) or `portal`.
    via TEXT NOT NULL DEFAULT 'staff',
    ticket_id INTEGER,
    ticket_number TEXT,
    -- `upheld` or `adjusted`, with the amount of the adjustment line.
    outcome TEXT,
    outcome_amount INTEGER,
    outcome_note TEXT,
    adjustment_line_id INTEGER,
    resolved_by INTEGER,
    resolved_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "logistics_statement_disputes_remittance" ON "logistics_statement_disputes" (remittance_id);
