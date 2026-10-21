-- 04 Accounting `ACC.RUL.11` (BL-ACC-02): the engagement letter sent for
-- acceptance in the portal, its version, the reminders and the chase task,
-- and the e-signature record (`DOC.SIG.01`, minimal: wl-documents has no
-- signature request yet).
CREATE TABLE "accounting_engagement_letters" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engagement_id INTEGER NOT NULL REFERENCES "accounting_engagements" (id),
    version INTEGER NOT NULL DEFAULT 1,
    sent_on TEXT NOT NULL,
    sent_by INTEGER,
    document_hash TEXT NOT NULL,
    signed_at TEXT,
    signed_on TEXT,
    signer_user_id INTEGER,
    signer_name TEXT,
    ip_address TEXT,
    superseded_on TEXT,
    reminded_7_on TEXT,
    reminded_14_on TEXT,
    chase_task_id INTEGER,
    chase_task_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_engagement_letters_engagement_id" ON "accounting_engagement_letters" (engagement_id);

ALTER TABLE "accounting_engagements" ADD COLUMN decline_note TEXT;
