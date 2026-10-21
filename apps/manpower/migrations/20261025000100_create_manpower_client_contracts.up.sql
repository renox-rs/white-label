-- The client contract (09 Manpower §6 *Client contract*, §7.1): the agreement
-- with a client company. The rate card is wl-invoicing's, found through the
-- kernel's `RateCards` by the contract as its source. The hold reasons and
-- the state before a hold (`hold`, `release`), the termination fields
-- (`terminate`) and the renewal come with the issues that build them.
CREATE TABLE "manpower_client_contracts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The reference the agency gives the contract.
    reference TEXT NOT NULL UNIQUE,
    client_party_id INTEGER NOT NULL,
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL,
    -- `weekly`, `fortnightly` or `monthly`.
    billing_cycle TEXT NOT NULL DEFAULT 'monthly',
    payment_terms_days INTEGER NOT NULL DEFAULT 30,
    -- Whether the agency must cover a worker's leave, suspension or rejection
    -- (`MPW.DEP.02`).
    cover_required INTEGER NOT NULL DEFAULT 0,
    -- Whole days after which a timesheet the client does not answer is
    -- deemed approved; none by default (`HRM.RUL.15`).
    deemed_approval_days INTEGER,
    -- The client contact the timesheet escalation goes to (a party).
    escalation_contact_party_id INTEGER,
    -- The calendar days of notice an early termination or a withdrawal
    -- needs.
    notice_period_days INTEGER NOT NULL DEFAULT 30,
    account_manager_user_id INTEGER,
    -- The signed contract (`DOC.SIG.01`): the day and the document.
    signed_on TEXT,
    signed_document TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    activated_on TEXT,
    cancel_reason TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_client_contracts_client" ON "manpower_client_contracts" (client_party_id);
CREATE INDEX "manpower_client_contracts_status" ON "manpower_client_contracts" (status);
