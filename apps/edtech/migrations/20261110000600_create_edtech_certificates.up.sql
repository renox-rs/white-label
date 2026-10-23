-- 10 EdTech certificate (`EDU.CRT.01`, 10 section 7.1 *Certificate*): proof of
-- completion of an enrolment. The number is assigned when it is issued (never
-- reused); the verification code is on the printed certificate and its QR and
-- opens the public page. `supersedes_id` and `superseded_by_id` link a
-- reissued certificate to the one it replaces.
CREATE TABLE "edtech_certificates" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    enrolment_id INTEGER NOT NULL REFERENCES "edtech_enrolments" (id),
    student_party_id INTEGER NOT NULL,
    student_name TEXT NOT NULL,
    course_title TEXT NOT NULL,
    approval_no TEXT,
    states_approval BOOLEAN NOT NULL DEFAULT FALSE,
    completion_date TEXT NOT NULL,
    number TEXT,
    verification_code TEXT NOT NULL,
    issued_on TEXT,
    issued_by INTEGER,
    supersedes_id INTEGER,
    superseded_by_id INTEGER,
    reissue_reason TEXT,
    revoke_reason TEXT,
    revoked_on TEXT,
    attestation_ref TEXT,
    status TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_certificates_number" ON "edtech_certificates" (number);
CREATE UNIQUE INDEX "edtech_certificates_verification_code" ON "edtech_certificates" (verification_code);
CREATE INDEX "edtech_certificates_enrolment_id" ON "edtech_certificates" (enrolment_id);
CREATE INDEX "edtech_certificates_status" ON "edtech_certificates" (status);
