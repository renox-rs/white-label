-- 08 Auto: the wait for parts and the substitute vehicle (`AUT.RUL.16`,
-- BL-AUT-04). A part line names where its part comes from and when it is
-- expected; the supply period per source is set at onboarding (7, 14 or 30
-- days); each wait for parts keeps its start, its legal supply date and which
-- steps of the daily rule fired; a warranty repair that takes longer than 7
-- days gets a substitute vehicle, or the customer's refusal with a reason.
ALTER TABLE "auto_job_part_lines" ADD COLUMN source TEXT NOT NULL DEFAULT 'stock';
ALTER TABLE "auto_job_part_lines" ADD COLUMN expected_date TEXT;

CREATE TABLE "auto_supply_periods" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    days INTEGER NOT NULL,
    updated_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_supply_periods_source" ON "auto_supply_periods" (source);

CREATE TABLE "auto_parts_waits" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    started_at TEXT NOT NULL,
    legal_date TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'stock',
    advisor_notified_at TEXT,
    customer_notified_at TEXT,
    legal_notified_at TEXT,
    ended_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "auto_parts_waits_job_card_id" ON "auto_parts_waits" (job_card_id);

CREATE TABLE "auto_substitutes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_card_id INTEGER NOT NULL REFERENCES "auto_job_cards" (id),
    requested_at TEXT NOT NULL,
    plate TEXT,
    from_date TEXT,
    to_date TEXT,
    refused_reason TEXT,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_substitutes_job_card_id" ON "auto_substitutes" (job_card_id);
