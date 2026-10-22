-- What a client contract said about timesheets from a moment on (09 Manpower §4.3, §6, BL-MPW-19):
-- the deemed-approval days and the escalation contact in force from `effective_from`. A timesheet is
-- deemed approved after the days in force when it was submitted, so a later change of the contract
-- does not move it. A contract nobody changed has no rows and its own columns apply.
CREATE TABLE "manpower_contract_terms" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_contract_id INTEGER NOT NULL REFERENCES "manpower_client_contracts" (id),
    deemed_approval_days INTEGER,
    escalation_contact_party_id INTEGER,
    effective_from TEXT NOT NULL,
    changed_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_contract_terms_contract" ON "manpower_contract_terms" (client_contract_id);
