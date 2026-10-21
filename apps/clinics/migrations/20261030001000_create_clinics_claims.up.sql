-- 05 Clinics claim lifecycle (`CLN.CLM.01`, `CLN.ACL.09`; base for BL-CLN-07
-- to BL-CLN-11): the clinic's claim wraps the draft payer bill a billed visit
-- makes. It keeps the coding, the submission attempts with the channel's
-- errors, the remittances and the clinic-side state; the money is the payer
-- bill's, in wl-invoicing.
CREATE TABLE "clinics_claims" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    visit_id INTEGER NOT NULL REFERENCES "clinics_visits" (id),
    payer_bill_id INTEGER NOT NULL,
    contract_id INTEGER,
    patient_party_id INTEGER NOT NULL,
    payer_party_id INTEGER NOT NULL,
    policy_id INTEGER,
    visit_date TEXT NOT NULL,
    payer_reference TEXT,
    claimed INTEGER NOT NULL DEFAULT 0,
    paid INTEGER NOT NULL DEFAULT 0,
    denied INTEGER NOT NULL DEFAULT 0,
    resubmission_count INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    submission_reference TEXT,
    submitted_on TEXT,
    late_reason TEXT,
    corrections TEXT,
    has_remittance BOOLEAN NOT NULL DEFAULT FALSE,
    last_remittance_on TEXT,
    last_remittance_reference TEXT,
    fix_deadline TEXT,
    held_kind TEXT,
    held_request_id INTEGER,
    held_reference TEXT,
    held_date TEXT,
    held_result TEXT,
    held_by INTEGER,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_claims_payer_bill_id" ON "clinics_claims" (payer_bill_id);
CREATE INDEX "clinics_claims_visit_id" ON "clinics_claims" (visit_id);
CREATE INDEX "clinics_claims_status" ON "clinics_claims" (status);

CREATE TABLE "clinics_claim_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    claim_id INTEGER NOT NULL REFERENCES "clinics_claims" (id),
    position INTEGER NOT NULL,
    payer_bill_line_id INTEGER,
    charge_line_id INTEGER,
    code TEXT NOT NULL,
    description TEXT NOT NULL,
    claimed INTEGER NOT NULL DEFAULT 0,
    paid INTEGER NOT NULL DEFAULT 0,
    denied INTEGER NOT NULL DEFAULT 0,
    recouped INTEGER NOT NULL DEFAULT 0,
    denial_code TEXT,
    denial_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_claim_lines_claim_id" ON "clinics_claim_lines" (claim_id);

CREATE TABLE "clinics_claim_attempts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    claim_id INTEGER NOT NULL REFERENCES "clinics_claims" (id),
    number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    reference TEXT,
    submitted_on TEXT NOT NULL,
    errors TEXT NOT NULL DEFAULT '[]',
    rejected_on TEXT,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_claim_attempts_claim_id_number" ON "clinics_claim_attempts" (claim_id, number);

CREATE TABLE "clinics_claim_remittances" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    claim_id INTEGER NOT NULL REFERENCES "clinics_claims" (id),
    kind TEXT NOT NULL,
    reference TEXT,
    advice_date TEXT NOT NULL,
    paid INTEGER NOT NULL DEFAULT 0,
    denied INTEGER NOT NULL DEFAULT 0,
    overpayment INTEGER NOT NULL DEFAULT 0,
    recouped INTEGER NOT NULL DEFAULT 0,
    code TEXT,
    reversal BOOLEAN NOT NULL DEFAULT FALSE,
    approval_request_id INTEGER,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_claim_remittances_claim_id" ON "clinics_claim_remittances" (claim_id);
