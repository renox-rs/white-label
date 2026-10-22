-- The deployment (09 Manpower §6 *Deployment*, §7.1): a worker placed at a
-- client site. The price comes from the rate card version in force
-- (`MPW.ACL.16`), so the deployment stores no bill rate. The suspension
-- reasons and dates (`suspend`) and the end date a notice replaced
-- (`give_notice`) come with the issues that build them.
CREATE TABLE "manpower_deployments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- A wl-hr employee id.
    employee_id INTEGER NOT NULL,
    client_contract_id INTEGER NOT NULL REFERENCES "manpower_client_contracts" (id),
    client_party_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL REFERENCES "manpower_sites" (id),
    -- The request it fills.
    request_id INTEGER REFERENCES "manpower_requests" (id),
    role TEXT NOT NULL,
    shift_type TEXT NOT NULL,
    shift_start TEXT,
    shift_end TEXT,
    start_date TEXT NOT NULL,
    end_date TEXT,
    -- `standard`, or `cover` for a worker who stands in for another
    -- (`MPW.DEP.02`), and the deployment covered.
    kind TEXT NOT NULL DEFAULT 'standard',
    covers_deployment_id INTEGER,
    status TEXT NOT NULL DEFAULT 'planned',
    activated_on TEXT,
    -- Why and when it ended (`end_date`, `contract_end`, `client_rejected`,
    -- `client_non_payment`, `replaced`, `exit`, `absconded`, `deceased`,
    -- `entered_in_error`).
    end_reason TEXT,
    ended_on TEXT,
    cancel_reason TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_deployments_employee" ON "manpower_deployments" (employee_id);
CREATE INDEX "manpower_deployments_contract" ON "manpower_deployments" (client_contract_id);
CREATE INDEX "manpower_deployments_request" ON "manpower_deployments" (request_id);
CREATE INDEX "manpower_deployments_status" ON "manpower_deployments" (status);
