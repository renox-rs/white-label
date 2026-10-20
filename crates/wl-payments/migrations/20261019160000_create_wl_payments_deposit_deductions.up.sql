-- The deductions of a deposit, published to the party in numbered versions, the
-- party's disputes of their lines, and what `forfeit` needs (wl-payments §4
-- *Deposit*, §5.4, issue #108). A published version is never changed: a
-- revision is the next version.
CREATE TABLE "wl_payments_deposit_deductions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deposit_id INTEGER NOT NULL REFERENCES "wl_payments_deposits" (id),
    -- 1, 2, ...: each revision publishes the next.
    version INTEGER NOT NULL,
    -- `deductions` or `forfeiture`.
    kind TEXT NOT NULL DEFAULT 'deductions',
    -- The reason of a forfeiture.
    reason TEXT,
    published_on TEXT NOT NULL,
    -- The last day of the dispute window (23:59 Asia/Dubai); none when the
    -- vertical does not allow a dispute.
    window_ends_on TEXT,
    published_by INTEGER,
    -- What the version deducts in all, in fils.
    total INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (deposit_id, version)
);

CREATE INDEX "wl_payments_deposit_deductions_deposit" ON "wl_payments_deposit_deductions" (deposit_id);

CREATE TABLE "wl_payments_deposit_deduction_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deduction_id INTEGER NOT NULL REFERENCES "wl_payments_deposit_deductions" (id),
    deposit_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    description TEXT NOT NULL,
    amount INTEGER NOT NULL,
    -- The reference of the evidence (inspection photos, a quote).
    evidence TEXT NOT NULL,
    billable INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_deposit_deduction_lines_deduction" ON "wl_payments_deposit_deduction_lines" (deduction_id);

-- One row per disputed line: the flag is set while one is `open`.
CREATE TABLE "wl_payments_deposit_disputes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deposit_id INTEGER NOT NULL REFERENCES "wl_payments_deposits" (id),
    deduction_id INTEGER NOT NULL REFERENCES "wl_payments_deposit_deductions" (id),
    line_id INTEGER NOT NULL REFERENCES "wl_payments_deposit_deduction_lines" (id),
    comment TEXT NOT NULL,
    raised_on TEXT NOT NULL,
    -- `open`, `revised` or `upheld`.
    status TEXT NOT NULL DEFAULT 'open',
    response TEXT,
    evidence TEXT,
    answered_by INTEGER,
    answered_on TEXT,
    -- The version a revision published.
    revised_in INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_deposit_disputes_deposit" ON "wl_payments_deposit_disputes" (deposit_id, status);

-- A legal case opened on the source agreement keeps the dispute flag until a
-- staff member records the decision.
ALTER TABLE "wl_payments_deposits" ADD COLUMN dispute_legal_case INTEGER NOT NULL DEFAULT 0;
-- `forfeit`: why, and the second user who approved it.
ALTER TABLE "wl_payments_deposits" ADD COLUMN forfeit_reason TEXT;
ALTER TABLE "wl_payments_deposits" ADD COLUMN forfeit_approved_by INTEGER;
