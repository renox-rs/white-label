-- 08 Auto insurance repair claim (`AUT.INS.01` .. `AUT.INS.04`, 08 section 6
-- *Insurance claim*, section 7.1 *Insurance repair claim*): one accident
-- repair funded by an insurer, with the policy and claim numbers, the police
-- report, the excess, the rate card version in force at registration, what the
-- insurer approved, and the payer bill and customer invoice of the split
-- billing. The job card it funds is `job_card_id`; the status moves only
-- through the machine.
CREATE TABLE "auto_insurance_claims" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    insurer_id INTEGER NOT NULL REFERENCES "auto_insurers" (id),
    customer_party_id INTEGER NOT NULL,
    vehicle_id INTEGER NOT NULL REFERENCES "auto_vehicles" (id),
    job_card_id INTEGER,
    policy_no TEXT NOT NULL,
    claim_no TEXT NOT NULL,
    accident_date TEXT NOT NULL,
    police_report_no TEXT,
    police_report_date TEXT,
    repair_type TEXT NOT NULL,
    excess INTEGER NOT NULL DEFAULT 0,
    rate_card_id INTEGER,
    rate_card_version INTEGER,
    estimate_amount INTEGER NOT NULL DEFAULT 0,
    approved_amount INTEGER NOT NULL DEFAULT 0,
    survey_booked_at TEXT,
    sent_to_insurer_at TEXT,
    submitted_at TEXT,
    approved_at TEXT,
    supplementary_at TEXT,
    completed_at TEXT,
    total_loss_at TEXT,
    total_loss_notice TEXT,
    withdrawal_document TEXT,
    closed_reason TEXT,
    closed_at TEXT,
    payer_bill_id INTEGER,
    customer_invoice_id INTEGER,
    invoiced_at TEXT,
    settled_at TEXT,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_insurance_claims_insurer_id_claim_no" ON "auto_insurance_claims" (insurer_id, claim_no);
CREATE INDEX "auto_insurance_claims_vehicle_id" ON "auto_insurance_claims" (vehicle_id);
CREATE INDEX "auto_insurance_claims_job_card_id" ON "auto_insurance_claims" (job_card_id);
CREATE INDEX "auto_insurance_claims_status" ON "auto_insurance_claims" (status);
