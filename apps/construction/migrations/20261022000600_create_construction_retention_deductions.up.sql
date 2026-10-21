-- What the client keeps of a retention part before it is invoiced (02
-- Construction §4.6, §7.1 *Retention* `deduct`, `CON.ACL.14`; BL-CON-08).
-- Money is fils. A deduction is applied to the part only when the project
-- manager (the MD above AED 25,000) accepts it.
CREATE TABLE "construction_retention_deductions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    retention_id INTEGER NOT NULL REFERENCES "construction_retentions" (id),
    amount INTEGER NOT NULL,
    -- The client's notice or contra-charge reference, and why.
    reference TEXT NOT NULL,
    reason TEXT NOT NULL,
    -- `undecided`, `accepted` or `contested`.
    status TEXT NOT NULL DEFAULT 'undecided',
    -- The QS who recorded it, and the project manager who decided it.
    recorded_by INTEGER NOT NULL,
    decided_by INTEGER,
    decided_at TEXT,
    -- When the amount was booked to the project's back-charge cost code.
    booked_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_retention_deductions_retention" ON "construction_retention_deductions" (retention_id);
