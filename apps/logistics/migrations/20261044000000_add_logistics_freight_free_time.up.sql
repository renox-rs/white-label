-- The free time of a container (03 Logistics §4.2 row *Container free time*,
-- §7.2 `LOG.RUL.14`; BL-LOG-24): when it was collected or returned empty, the
-- daily log of the days past an end date, the notices already sent for an end
-- date, and what a supplier bill charge records about the container.
ALTER TABLE "logistics_freight_containers" ADD COLUMN collected_on TEXT;
ALTER TABLE "logistics_freight_containers" ADD COLUMN returned_empty_on TEXT;

-- One line for each day past an end date, for each container and charge type.
CREATE TABLE "logistics_freight_free_time_log" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    container_id INTEGER NOT NULL REFERENCES "logistics_freight_containers" (id),
    -- `port_storage`, `demurrage` or `detention`.
    charge_type TEXT NOT NULL,
    log_date TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_freight_free_time_log_date" ON "logistics_freight_free_time_log" (container_id, charge_type, log_date);
CREATE INDEX "logistics_freight_free_time_log_job" ON "logistics_freight_free_time_log" (job_id);

-- A notice sent: once for each container, charge type, end date and moment.
CREATE TABLE "logistics_freight_free_time_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    container_id INTEGER NOT NULL REFERENCES "logistics_freight_containers" (id),
    charge_type TEXT NOT NULL,
    end_date TEXT NOT NULL,
    -- `before` (N days before the end date) or `on` (on it).
    moment TEXT NOT NULL,
    sent_on TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_freight_free_time_notices_once" ON "logistics_freight_free_time_notices" (container_id, charge_type, end_date, moment);

-- A container charge of a supplier bill allocated to the job.
ALTER TABLE "logistics_job_lines" ADD COLUMN container_id INTEGER;
ALTER TABLE "logistics_job_lines" ADD COLUMN charge_type TEXT;
ALTER TABLE "logistics_job_lines" ADD COLUMN billed_days INTEGER;
-- Why the cost has its liability: late documents, late duty payment, late
-- collection by the client, company delay or other.
ALTER TABLE "logistics_job_lines" ADD COLUMN liability_reason TEXT;
