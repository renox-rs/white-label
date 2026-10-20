-- 04 Accounting `ACC.ACL.14` (BL-ACC-07): the neutral template the MLRO
-- chose to tell the client about a restricted suspension or end, recorded on
-- the engagement.
CREATE TABLE "accounting_client_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engagement_id INTEGER NOT NULL REFERENCES "accounting_engagements" (id),
    template TEXT NOT NULL,
    sent_by INTEGER NOT NULL,
    sent_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_client_notices_engagement_id" ON "accounting_client_notices" (engagement_id);
