-- 04 Accounting `ACC.ACL.09` (BL-ACC-06): a write-off of unbilled time (WIP)
-- of an engagement; above a share of the WIP a manager's needs a partner.
CREATE TABLE "accounting_wip_write_offs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engagement_id INTEGER NOT NULL REFERENCES "accounting_engagements" (id),
    amount INTEGER NOT NULL,
    wip_amount INTEGER NOT NULL,
    reason TEXT NOT NULL,
    requested_by INTEGER,
    written_off_on TEXT,
    status TEXT NOT NULL DEFAULT 'proposed',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_wip_write_offs_engagement_id" ON "accounting_wip_write_offs" (engagement_id);
