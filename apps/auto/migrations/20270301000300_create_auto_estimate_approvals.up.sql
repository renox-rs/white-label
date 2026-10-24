-- 08 Auto estimate approval (`DOC.SIG.01`, `FIN.QUO.01`, `AUT.JOB.02`): one
-- round of an estimate sent to the customer, with the link token the customer
-- opens. The customer's consent to the cost, the time and the warranty is
-- recorded when it is approved: who, how (link, signature, recorded by the
-- advisor) and when.
CREATE TABLE "auto_estimate_approvals" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    revision INTEGER NOT NULL,
    token TEXT NOT NULL,
    net_total INTEGER NOT NULL DEFAULT 0,
    promised_for TEXT,
    warranty_days INTEGER NOT NULL DEFAULT 15,
    status TEXT NOT NULL,
    sent_by INTEGER,
    sent_at TEXT,
    decided_at TEXT,
    decided_by_name TEXT,
    decided_by_user INTEGER,
    method TEXT,
    signature TEXT,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_estimate_approvals_token" ON "auto_estimate_approvals" (token);
CREATE INDEX "auto_estimate_approvals_job_card_id" ON "auto_estimate_approvals" (job_card_id);
