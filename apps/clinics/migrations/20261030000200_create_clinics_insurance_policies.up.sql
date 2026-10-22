-- 05 Clinics (BL-CLN-01 base): a patient's insurance policy and the
-- eligibility results recorded for it (`CLN.INS.02`). The card image is not
-- kept here: it needs `patients.sensitive.view` (`CLN.ACL.12`) and comes with
-- the portal upload (BL-CLN-04).
CREATE TABLE "clinics_insurance_policies" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    patient_party_id INTEGER NOT NULL,
    payer_party_id INTEGER NOT NULL,
    plan TEXT NOT NULL,
    member_id TEXT NOT NULL,
    valid_from TEXT NOT NULL,
    valid_to TEXT,
    card_valid_to TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_insurance_policies_patient_party_id" ON "clinics_insurance_policies" (patient_party_id);

CREATE TABLE "clinics_eligibility_checks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    policy_id INTEGER NOT NULL REFERENCES "clinics_insurance_policies" (id),
    checked_on TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    positive BOOLEAN NOT NULL DEFAULT FALSE,
    reference TEXT,
    note TEXT,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_eligibility_checks_policy_id" ON "clinics_eligibility_checks" (policy_id);
