-- The Project (02 Construction §6, §7.1): an awarded contract, with the
-- termination it may end with. Money is fils.
CREATE TABLE "construction_projects" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    -- The client or main contractor, a wl-parties party.
    client_party_id INTEGER NOT NULL,
    contract_value INTEGER NOT NULL DEFAULT 0,
    budget INTEGER NOT NULL DEFAULT 0,
    -- The retention withheld on each IPA, in basis points, its cap and the
    -- share of the first release part.
    retention_bps INTEGER NOT NULL DEFAULT 0,
    retention_cap INTEGER,
    retention_first_bps INTEGER NOT NULL DEFAULT 5000,
    -- Whether both retention parts become due when the project is
    -- terminated (`⚠ open: lawyer (L-37)`).
    retention_due_on_termination INTEGER NOT NULL DEFAULT 1,
    defects_liability_days INTEGER NOT NULL DEFAULT 365,
    -- The project manager, who gets the tasks of the project.
    manager_user_id INTEGER,
    status TEXT NOT NULL DEFAULT 'setup',
    hold_reason TEXT,
    -- The termination: why (`terminated_by_client`, `our_default`,
    -- `main_contractor_insolvent`, `agreed`), the notice, the last working
    -- day, who asked and when it took effect.
    termination_reason TEXT,
    termination_notice TEXT,
    termination_effective_on TEXT,
    termination_requested_by INTEGER,
    terminated_at TEXT,
    -- When the wind-down after the commit ran (tasks, requests, count...).
    wound_down_at TEXT,
    cancel_reason TEXT,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_projects_status" ON "construction_projects" (status);
