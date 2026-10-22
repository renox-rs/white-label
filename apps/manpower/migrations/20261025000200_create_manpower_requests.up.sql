-- The manpower request (09 Manpower §6 *Manpower request*, §7.1) and the
-- cover request of `MPW.DEP.02`, which is a request of the kind
-- `replacement`: the client's order for workers, or the agency's own order
-- for a worker to stand in for one on leave, suspended or rejected.
CREATE TABLE "manpower_requests" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_contract_id INTEGER NOT NULL REFERENCES "manpower_client_contracts" (id),
    client_party_id INTEGER NOT NULL,
    site_id INTEGER NOT NULL REFERENCES "manpower_sites" (id),
    role TEXT NOT NULL,
    headcount INTEGER NOT NULL,
    -- The shift pattern: the shift type the rate card prices (`day`,
    -- `night`...) and the shift times as `HH:MM` (Asia/Dubai).
    shift_type TEXT NOT NULL,
    shift_start TEXT,
    shift_end TEXT,
    start_date TEXT NOT NULL,
    -- Empty for an open-ended order (a cover for a suspension).
    end_date TEXT,
    -- `standard` or `replacement`.
    kind TEXT NOT NULL DEFAULT 'standard',
    -- A replacement: why (`leave`, `suspension`, `rejection`), what asked for
    -- it (`hr.leave_request:5`; one cover request per cause) and the
    -- deployment it stands in for.
    cause TEXT,
    cause_ref TEXT,
    replaces_deployment_id INTEGER,
    -- How many deployments were activated for it; a deployment that ends
    -- does not reopen the request.
    filled_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'draft',
    submitted_by INTEGER,
    submitted_at TEXT,
    cancel_reason TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_requests_contract" ON "manpower_requests" (client_contract_id);
CREATE INDEX "manpower_requests_status" ON "manpower_requests" (status);
CREATE UNIQUE INDEX "manpower_requests_cause" ON "manpower_requests" (cause_ref) WHERE cause_ref IS NOT NULL;
