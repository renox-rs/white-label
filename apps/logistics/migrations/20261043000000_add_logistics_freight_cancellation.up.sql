-- Cancelling a freight job before departure (03 Logistics §4.2 row *Client
-- cancels before departure*, §7.1 row `cancel`, §9 *VAT*; BL-LOG-23): the
-- quotation's terms as the officer enters them (the stand-in for the
-- quotation, `FIN.QUO.01`), what each cancellation did, and the accountant's
-- decision on a cost billed after the cancel.
CREATE TABLE "logistics_freight_quotations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    reference TEXT NOT NULL DEFAULT '',
    -- One amount for any stage, in fils.
    fixed_charge INTEGER,
    -- One amount per stage, in fils.
    booked_charge INTEGER,
    documents_pending_charge INTEGER,
    -- Whether the costs already incurred are recharged to the client.
    recharge_incurred INTEGER NOT NULL DEFAULT 1,
    entered_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_freight_quotations_job" ON "logistics_freight_quotations" (job_id);

CREATE TABLE "logistics_freight_cancellations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES "logistics_freight_jobs" (id),
    -- The job's state at `cancel`: `booked` or `documents_pending`.
    stage TEXT NOT NULL,
    reason TEXT NOT NULL,
    -- Whether the quotation had a cancellation charge for the stage.
    terms_found INTEGER NOT NULL DEFAULT 0,
    quotation_ref TEXT NOT NULL DEFAULT '',
    -- The cancellation charge and the incurred costs recharged, net, in fils.
    charge INTEGER NOT NULL DEFAULT 0,
    recharged INTEGER NOT NULL DEFAULT 0,
    -- The costs already incurred that are not recharged, in fils: the job's loss.
    absorbed INTEGER NOT NULL DEFAULT 0,
    invoice_id INTEGER,
    -- The documentation officer's task when nothing was drafted.
    ticket_id INTEGER,
    cancelled_by INTEGER,
    cancelled_at TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "logistics_freight_cancellations_job" ON "logistics_freight_cancellations" (job_id);

-- What the accountant decided for a cost billed after the cancel:
-- `invoiced` or `absorbed`; none while it waits.
ALTER TABLE "logistics_job_lines" ADD COLUMN decision TEXT;
