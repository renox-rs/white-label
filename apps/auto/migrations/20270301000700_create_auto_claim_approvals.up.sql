-- 08 Auto insurer approval (`AUT.INS.02`, `AUT.INS.03`, 08 section 6
-- *Insurer approval*): the estimate sent to the insurer, a supplementary for
-- hidden damage, or the charges of a total loss, and the insurer's answer: the
-- reference or LPO, the approved amount per line and the document it came in.
-- `job_round_id` is the estimate round of the job card the request is about.
CREATE TABLE "auto_claim_approvals" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    claim_id INTEGER NOT NULL REFERENCES "auto_insurance_claims" (id),
    kind TEXT NOT NULL,
    round_no INTEGER NOT NULL DEFAULT 1,
    job_round_id INTEGER,
    estimate_amount INTEGER NOT NULL DEFAULT 0,
    findings TEXT,
    outcome TEXT NOT NULL,
    reference TEXT,
    approved_amount INTEGER NOT NULL DEFAULT 0,
    lines TEXT NOT NULL DEFAULT '[]',
    document TEXT,
    reason TEXT,
    requested_by INTEGER,
    requested_at TEXT,
    decided_by INTEGER,
    decided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_claim_approvals_claim_id" ON "auto_claim_approvals" (claim_id);
