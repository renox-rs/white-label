-- 05 Clinics `CLN.ACL.15` (BL-CLN-01): the data the front desk records
-- before check-in (the eligibility result, the coverage outcome, the policy
-- of a re-split, the self-pay consent), moved to the visit by the check-in;
-- the self-pay consent with the patient's acknowledgement (`DOC.SIG.01`,
-- minimal: wl-documents has no signature request yet); the coverage columns
-- of the visit.
CREATE TABLE "clinics_pending_check_ins" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL,
    appointment_id INTEGER NOT NULL REFERENCES "clinics_appointments" (id),
    policy_id INTEGER NOT NULL,
    visit_date TEXT NOT NULL,
    eligibility_check_id INTEGER,
    eligibility_positive BOOLEAN,
    outcome TEXT,
    coverage_policy_id INTEGER,
    consent_id INTEGER,
    moved_visit_id INTEGER,
    discarded_on TEXT,
    discard_reason TEXT,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_pending_check_ins_booking_id" ON "clinics_pending_check_ins" (booking_id);

CREATE TABLE "clinics_self_pay_consents" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    booking_id INTEGER NOT NULL,
    patient_party_id INTEGER,
    pending_id INTEGER,
    visit_id INTEGER,
    lines TEXT NOT NULL DEFAULT '[]',
    estimated_fils INTEGER NOT NULL DEFAULT 0,
    priced_on TEXT NOT NULL,
    document_hash TEXT NOT NULL,
    signed_at TEXT NOT NULL,
    signed_on TEXT NOT NULL,
    signer_name TEXT NOT NULL,
    signer_user_id INTEGER,
    ip_address TEXT,
    superseded_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_self_pay_consents_booking_id" ON "clinics_self_pay_consents" (booking_id);

ALTER TABLE "clinics_visits" ADD COLUMN eligibility_check_id INTEGER;
ALTER TABLE "clinics_visits" ADD COLUMN eligibility_positive BOOLEAN;
ALTER TABLE "clinics_visits" ADD COLUMN coverage_outcome TEXT;
ALTER TABLE "clinics_visits" ADD COLUMN coverage_policy_id INTEGER;
ALTER TABLE "clinics_visits" ADD COLUMN consent_id INTEGER;
