-- 04 Accounting §6 *Return*, §7.1 *Return* (base for BL-ACC-16, BL-ACC-17,
-- BL-ACC-06): a VAT or corporate tax return of a client for a period, and the
-- signatures its approvals record (`DOC.SIG.01`, minimal record until
-- wl-documents has the signature request).
CREATE TABLE "accounting_returns" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_party_id INTEGER NOT NULL,
    engagement_id INTEGER NOT NULL REFERENCES "accounting_engagements" (id),
    task_id INTEGER,
    return_type TEXT NOT NULL,
    period_start TEXT,
    period_end TEXT NOT NULL,
    due_date TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    supersedes_id INTEGER,
    preparer_user_id INTEGER,
    reviewer_user_id INTEGER,
    figures TEXT,
    workpapers_attached BOOLEAN NOT NULL DEFAULT FALSE,
    send_back_comment TEXT,
    cancel_reason TEXT,
    client_messages_suppressed BOOLEAN NOT NULL DEFAULT FALSE,
    filed_on TEXT,
    filed_by_user_id INTEGER,
    filed_by_client BOOLEAN NOT NULL DEFAULT FALSE,
    tax_agent_registration TEXT,
    acknowledgement TEXT,
    correction_route TEXT,
    correction_reference TEXT,
    correction_reason TEXT,
    status TEXT NOT NULL DEFAULT 'preparing',
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_returns_engagement_id" ON "accounting_returns" (engagement_id);
CREATE INDEX "accounting_returns_client_party_id" ON "accounting_returns" (client_party_id);
CREATE INDEX "accounting_returns_status" ON "accounting_returns" (status);

CREATE TABLE "accounting_return_signatures" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    return_id INTEGER NOT NULL REFERENCES "accounting_returns" (id),
    signer_user_id INTEGER NOT NULL,
    signer_name TEXT NOT NULL,
    signed_at TEXT NOT NULL,
    ip_address TEXT,
    document_hash TEXT NOT NULL,
    withdrawn_at TEXT,
    withdrawn_by INTEGER,
    withdrawn_reason TEXT,
    withdrawn_instruction TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "accounting_return_signatures_return_id" ON "accounting_return_signatures" (return_id);
