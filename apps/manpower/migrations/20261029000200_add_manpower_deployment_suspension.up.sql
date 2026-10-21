-- The suspension of a deployment (09 Manpower §7.1 *Deployment* `suspend`,
-- §7.2 `MPW.RUL.12`, BL-MPW-11): the day it began, the day it ended and the
-- calendar days of the earlier suspensions (billing does not count them), when
-- the client, the account manager and the cover request were taken care of
-- after the commit, and one row per reason (an expired document kind or a
-- reason the account manager gave).
ALTER TABLE "manpower_deployments" ADD COLUMN suspended_on TEXT;
ALTER TABLE "manpower_deployments" ADD COLUMN resumed_on TEXT;
ALTER TABLE "manpower_deployments" ADD COLUMN suspended_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "manpower_deployments" ADD COLUMN suspension_announced_at TEXT;

CREATE TABLE "manpower_deployment_suspensions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id INTEGER NOT NULL REFERENCES "manpower_deployments" (id),
    -- `document` (an expired kind) or `manual` (site access withdrawn,
    -- investigation...).
    kind TEXT NOT NULL,
    -- `document`: the document kind's key; `manual`: what the account manager
    -- wrote.
    reason TEXT NOT NULL,
    -- `document`: the expiry date; the kind was valid through it.
    expired_on TEXT,
    added_on TEXT NOT NULL,
    -- The user who suspended; none when the rule did.
    added_by INTEGER,
    -- Set when the cause is gone: a renewed document was accepted, or the
    -- deployment resumed.
    cleared_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_deployment_suspensions_deployment"
    ON "manpower_deployment_suspensions" (deployment_id);
