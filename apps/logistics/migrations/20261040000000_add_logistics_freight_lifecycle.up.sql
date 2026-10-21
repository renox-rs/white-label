-- The freight job lifecycle (03 Logistics §6 *Freight job*, §7.1 *Freight job*,
-- `LOG.JOB.01`; base for BL-LOG-21, BL-LOG-22, BL-LOG-23, BL-LOG-24): the
-- move, its documents and milestones, the containers, the stand-in for job
-- costing (`FIN.CST.01`) and the margin at each close.
ALTER TABLE "logistics_freight_jobs" ADD COLUMN mode TEXT NOT NULL DEFAULT 'road';
ALTER TABLE "logistics_freight_jobs" ADD COLUMN origin TEXT NOT NULL DEFAULT '';
ALTER TABLE "logistics_freight_jobs" ADD COLUMN destination TEXT NOT NULL DEFAULT '';
ALTER TABLE "logistics_freight_jobs" ADD COLUMN bl_awb TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN declaration_no TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN duties INTEGER;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN delivery_order_ref TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN eta TEXT;
-- International with the client contracted for the whole move (VAT, §9).
ALTER TABLE "logistics_freight_jobs" ADD COLUMN international INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN pod_ref TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN cancel_reason TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN documents_note TEXT;
-- The hold: why, the note the client reads, the state before the hold.
ALTER TABLE "logistics_freight_jobs" ADD COLUMN hold_reason TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN hold_note TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN held_from TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN held_at TEXT;
ALTER TABLE "logistics_freight_jobs" ADD COLUMN closed_at TEXT;

-- The first status of a job opened before the lifecycle was `open`.
UPDATE "logistics_freight_jobs" SET status = 'booked' WHERE status = 'open';

-- A container of a job with its free-time end dates.
CREATE TABLE "logistics_freight_containers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    number TEXT NOT NULL,
    size TEXT NOT NULL DEFAULT '',
    port_storage_end TEXT,
    demurrage_end TEXT,
    detention_end TEXT,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_freight_containers_number" ON "logistics_freight_containers" (job_id, number);

-- The milestones and the other events of a job, oldest first.
CREATE TABLE "logistics_freight_events" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    kind TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    occurred_at TEXT NOT NULL,
    actor_user_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "logistics_freight_events_job" ON "logistics_freight_events" (job_id);

-- A revenue or cost line of a job: the stand-in for job costing
-- (`FIN.CST.01`), supplier bills (`FIN.PAB.01`) and disbursements
-- (`FIN.DSB.01`). Money is fils.
CREATE TABLE "logistics_job_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    -- `revenue` or `cost`.
    kind TEXT NOT NULL,
    description TEXT NOT NULL,
    amount INTEGER NOT NULL,
    -- For a cost: `client` or `company`, none until it is allocated.
    liability TEXT,
    allocated INTEGER NOT NULL DEFAULT 0,
    rechargeable INTEGER NOT NULL DEFAULT 0,
    -- For a revenue line: whether its invoice is issued.
    issued INTEGER NOT NULL DEFAULT 0,
    invoice_id INTEGER,
    -- A supplier bill or disbursement this cost comes from.
    source_ref TEXT,
    -- Recorded after the job was closed (a late cost).
    late INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE INDEX "logistics_job_lines_job" ON "logistics_job_lines" (job_id);

-- The margin at each close of a job; reports show the latest.
CREATE TABLE "logistics_freight_margins" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    close_no INTEGER NOT NULL,
    revenue INTEGER NOT NULL,
    cost INTEGER NOT NULL,
    margin INTEGER NOT NULL,
    closed_by INTEGER,
    closed_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_freight_margins_close" ON "logistics_freight_margins" (job_id, close_no);
