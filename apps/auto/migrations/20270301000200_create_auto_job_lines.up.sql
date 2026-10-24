-- 08 Auto estimate lines (`AUT.JOB.01`, `AUT.JOB.02`, `AUT.JOB.03`): the
-- labour of a job card (hourly, or a flat-rate menu operation) and its parts
-- (new, used or renovated, special-order or not). A line is drafted, sent to
-- the customer with an estimate, then approved or declined; the work done is
-- recorded per line.
CREATE TABLE "auto_job_labour_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    operation_id INTEGER,
    description TEXT NOT NULL,
    pricing TEXT NOT NULL,
    hours_milli INTEGER NOT NULL DEFAULT 0,
    rate INTEGER NOT NULL DEFAULT 0,
    approval TEXT NOT NULL,
    approval_id INTEGER,
    work TEXT NOT NULL,
    hours_done_milli INTEGER NOT NULL DEFAULT 0,
    done_at TEXT,
    done_by INTEGER,
    line_no INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_job_labour_lines_job_card_id" ON "auto_job_labour_lines" (job_card_id);

CREATE TABLE "auto_job_part_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    item_code TEXT,
    description TEXT NOT NULL,
    quantity_milli INTEGER NOT NULL DEFAULT 1000,
    unit_price INTEGER NOT NULL DEFAULT 0,
    part_condition TEXT NOT NULL,
    special_order BOOLEAN NOT NULL DEFAULT FALSE,
    special_order_agreed BOOLEAN NOT NULL DEFAULT FALSE,
    approval TEXT NOT NULL,
    approval_id INTEGER,
    store_location_id INTEGER,
    issued_milli INTEGER NOT NULL DEFAULT 0,
    posted_milli INTEGER NOT NULL DEFAULT 0,
    short_milli INTEGER NOT NULL DEFAULT 0,
    fitted_milli INTEGER NOT NULL DEFAULT 0,
    returned_milli INTEGER NOT NULL DEFAULT 0,
    line_no INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_job_part_lines_job_card_id" ON "auto_job_part_lines" (job_card_id);
