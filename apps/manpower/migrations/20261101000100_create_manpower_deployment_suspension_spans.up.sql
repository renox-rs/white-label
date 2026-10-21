-- The runs of days a deployment was suspended (09 Manpower §7.1 *Deployment*
-- `suspend` and `resume`, §4.3, BL-MPW-11, MPW.RUL.05): one row per suspension,
-- from the day it began to the day before the worker was back (`to_on` is the
-- day of the `resume`, so the days suspended are `to_on` - `from_on`; none
-- while the deployment is still suspended). Billing leaves these days out
-- whatever the unit of the rate. Deployments suspended before this table are
-- given the suspension their columns still remember.
CREATE TABLE "manpower_deployment_suspension_spans" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deployment_id INTEGER NOT NULL REFERENCES "manpower_deployments" (id),
    from_on TEXT NOT NULL,
    to_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_deployment_suspension_spans_deployment"
    ON "manpower_deployment_suspension_spans" (deployment_id);

INSERT INTO "manpower_deployment_suspension_spans" (deployment_id, from_on, to_on, created_at, updated_at)
SELECT id, suspended_on,
       CASE WHEN status = 'suspended' OR resumed_on IS NULL OR resumed_on < suspended_on
            THEN NULL ELSE resumed_on END,
       CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
FROM "manpower_deployments"
WHERE suspended_on IS NOT NULL;
