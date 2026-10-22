-- The documents that will expire during a deployment (09 Manpower §4.5 step 3,
-- BL-MPW-09, `DOC.RUL.02`): the nightly look-ahead of wl-documents lists the
-- deployments an expiring document affects, and each is flagged *document
-- expiring <kind> <date>* until the renewed document is accepted.
CREATE TABLE "manpower_deployment_document_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id INTEGER NOT NULL REFERENCES "manpower_deployments" (id),
    -- The expiring document and its kind.
    document_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    expires_on TEXT NOT NULL,
    -- The day a renewed document was accepted.
    cleared_on TEXT,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (deployment_id, document_id)
);

CREATE INDEX "manpower_deployment_document_flags_deployment"
    ON "manpower_deployment_document_flags" (deployment_id);
