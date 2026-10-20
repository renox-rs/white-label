-- The deposit deductions of a lease after they are published (01 Property
-- `PRP.RUL.11`, BL-PRP-16): the vertical keeps one row per deposit, fed by the
-- events of wl-payments, so that the daily rule can count the dispute window
-- from the latest published version and the escalation from the day the
-- dispute was opened.
CREATE TABLE "property_deposit_watches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The id of version 1: the lease the deposit is held for.
    root_id INTEGER NOT NULL,
    deposit_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    -- The latest published version, and the day it was published.
    version INTEGER NOT NULL DEFAULT 1,
    published_on TEXT NOT NULL,
    window_ends_on TEXT,
    -- The day the accountant was told that the settlement may proceed.
    notified_on TEXT,
    -- The day the open dispute was opened; empty when none is open.
    dispute_opened_on TEXT,
    -- The day the open dispute was escalated.
    escalated_on TEXT,
    -- The day the deposit was settled.
    closed_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "property_deposit_watches_deposit" ON "property_deposit_watches" (deposit_id);
CREATE INDEX "property_deposit_watches_root" ON "property_deposit_watches" (root_id);
