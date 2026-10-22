-- 10 EdTech enrolment (`EDU.ENR.01`, 10 section 7.1 *Enrolment*): a student in
-- a batch, a session package or a membership. The student, the guardian and
-- the payer are parties (wl-parties, by id). Amounts are fils with VAT.
-- `seat_held` says the batch's seat is counted for this enrolment (taken at
-- creation, given back by `lapse`, `withdraw`, `complete` and `transfer`).
CREATE TABLE "edtech_enrolments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    batch_id INTEGER REFERENCES "edtech_batches" (id),
    student_party_id INTEGER NOT NULL,
    guardian_party_id INTEGER,
    payer_party_id INTEGER NOT NULL,
    campus_type TEXT,
    campus_id INTEGER,
    counsellor_id INTEGER,
    item_code TEXT,
    item_name TEXT,
    price INTEGER NOT NULL DEFAULT 0,
    discount INTEGER NOT NULL DEFAULT 0,
    discount_approved BOOLEAN NOT NULL DEFAULT TRUE,
    registration_fee INTEGER NOT NULL DEFAULT 0,
    admin_fee INTEGER NOT NULL DEFAULT 0,
    sessions_total INTEGER NOT NULL DEFAULT 0,
    instalments INTEGER NOT NULL DEFAULT 1,
    policy TEXT NOT NULL DEFAULT '{}',
    terms_accepted_at TEXT,
    seat_held BOOLEAN NOT NULL DEFAULT FALSE,
    seat_hold_until TEXT,
    first_invoice_id INTEGER,
    package_id INTEGER,
    subscription_id INTEGER,
    refund_amount INTEGER NOT NULL DEFAULT 0,
    amount_owed INTEGER NOT NULL DEFAULT 0,
    withdrawn_on TEXT,
    withdrawal_reason TEXT,
    completed_on TEXT,
    status TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "edtech_enrolments_batch_id" ON "edtech_enrolments" (batch_id);
CREATE INDEX "edtech_enrolments_student_party_id" ON "edtech_enrolments" (student_party_id);
CREATE INDEX "edtech_enrolments_first_invoice_id" ON "edtech_enrolments" (first_invoice_id);
CREATE INDEX "edtech_enrolments_status" ON "edtech_enrolments" (status);
