-- The recall of an ingredient lot traced to customers (06 F&B §4.4 recall row,
-- §9 *Recalls*; BL-FNB-13): one notice to each customer of a direct order
-- listing all their possible orders, and the owner's decision on a public
-- notice for the sales without a known customer, one per recall.
CREATE TABLE "fnb_recall_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The recall (wl-inventory) and the customer told.
    recall_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    order_numbers TEXT NOT NULL DEFAULT '',
    line_count INTEGER NOT NULL DEFAULT 0,
    sent_by INTEGER,
    sent_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "fnb_recall_notices_recall" ON "fnb_recall_notices" (recall_id);
CREATE INDEX "fnb_recall_notices_party" ON "fnb_recall_notices" (party_id);

CREATE TABLE "fnb_recall_decisions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recall_id INTEGER NOT NULL,
    -- `public_notice` or `no_public_notice`.
    decision TEXT NOT NULL,
    aggregators_told INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT '',
    decided_by INTEGER NOT NULL,
    decided_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_recall_decisions_recall" ON "fnb_recall_decisions" (recall_id);
