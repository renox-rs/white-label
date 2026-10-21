-- The calls on a guarantee (02 Construction §4.6 row "The client calls a
-- guarantee", §7.1 *Guarantee* `call` and `settle_call`; BL-CON-10). Money is
-- fils. A call is open until the bank's debit advice, or the beneficiary's
-- withdrawal letter, settles it.
CREATE TABLE "construction_guarantee_calls" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guarantee_id INTEGER NOT NULL REFERENCES "construction_guarantees" (id),
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- The beneficiary's demand notice, a document reference, and the amount
    -- it demands.
    demand_notice TEXT NOT NULL,
    amount INTEGER NOT NULL,
    called_by INTEGER,
    -- The bank's debit advice, or the withdrawal letter when nothing was
    -- paid, and the amount the bank paid.
    settlement_document TEXT,
    amount_paid INTEGER,
    -- Set by the project manager: the paid amount is a claim in the final
    -- account.
    disputed INTEGER NOT NULL DEFAULT 0,
    settled_at TEXT,
    -- When the paid amount was booked to the called-guarantees cost code.
    booked_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_guarantee_calls_guarantee" ON "construction_guarantee_calls" (guarantee_id);
