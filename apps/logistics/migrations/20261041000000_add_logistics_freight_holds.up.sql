-- Holding and releasing a freight job (03 Logistics §4.2 rows *Customs hold...*
-- and *Mirsal or ATLP down*, §7.1 rows `hold` and `release`, §7.2
-- `LOG.RUL.09`; BL-LOG-21): each hold period with its reason, its note and the
-- state before it, and the ETA the alert was last raised for.
CREATE TABLE "logistics_freight_holds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    reason TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    -- The state before the hold: where `release` returns to.
    from_state TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    -- How the reason was resolved.
    release_note TEXT,
    -- Further reasons added while the job was on hold, one per line.
    extra_notes TEXT NOT NULL DEFAULT '',
    held_by INTEGER,
    released_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "logistics_freight_holds_job" ON "logistics_freight_holds" (job_id);

-- The ETA for which `LOG.RUL.09` last told the documentation officer.
ALTER TABLE "logistics_freight_jobs" ADD COLUMN eta_alerted_for TEXT;
