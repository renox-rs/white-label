-- The typed deductions a client makes on a certificate (02 Construction §6,
-- §7.1 *IPA*, `CON.ACL.14`). Money is fils, excluding VAT.
CREATE TABLE "construction_ipa_deductions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    ipa_id INTEGER NOT NULL REFERENCES "construction_ipas" (id),
    -- `back_charge`, `contra_charge`, `liquidated_damages` or `other`.
    kind TEXT NOT NULL,
    -- The client's reference, e.g. `BC-12`.
    reference TEXT NOT NULL,
    -- Needed for the kind `other`.
    description TEXT NOT NULL DEFAULT '',
    amount INTEGER NOT NULL,
    -- `undecided`, `accepted` or `contested`.
    status TEXT NOT NULL DEFAULT 'undecided',
    -- The QS who recorded it, and the project manager who decided it.
    recorded_by INTEGER NOT NULL,
    decided_by INTEGER,
    decided_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_ipa_deductions_ipa" ON "construction_ipa_deductions" (ipa_id);
