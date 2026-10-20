-- The dispute flag of an owner statement and the disputes behind it (01
-- Property §7.1 *Owner statement* "Dispute flag", `PRP.ACL.21`; BL-PRP-21): an
-- owner disputes a line of a published or settled statement; the flag stays
-- until every dispute is cleared by a revision or an outcome.
ALTER TABLE "property_owner_statements" ADD COLUMN disputed INTEGER NOT NULL DEFAULT 0;

CREATE TABLE "property_statement_disputes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    statement_id INTEGER NOT NULL REFERENCES "property_owner_statements" (id),
    -- The line disputed, when the owner names one.
    line_id INTEGER,
    -- What is disputed: the line's text and amount, as the owner saw them.
    description TEXT NOT NULL,
    -- The owner's words, as staff took them.
    comment TEXT NOT NULL,
    -- open or cleared.
    status TEXT NOT NULL DEFAULT 'open',
    -- upheld, adjusted, reallocated or revised, once cleared.
    outcome TEXT,
    -- The adjustment on this owner's balance, in fils.
    adjustment INTEGER,
    -- The other agreement of a cost allocated to the wrong owner.
    other_agreement_id INTEGER,
    note TEXT,
    ticket_id INTEGER,
    ticket_number TEXT,
    opened_by INTEGER,
    opened_on TEXT NOT NULL,
    cleared_by INTEGER,
    cleared_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_statement_disputes_statement" ON "property_statement_disputes" (statement_id, status);
