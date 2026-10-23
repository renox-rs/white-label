-- 10 EdTech freeze cause (10 section 7.1 *Enrolment*, `freeze`/`unfreeze`): each
-- cause that holds an enrolment `frozen` is stored; the enrolment is
-- unfrozen only when none is open. `counted_days` is what the cause counts
-- against the allowance of 30 days a year (0 for a block or a mirrored
-- system freeze).
CREATE TABLE "edtech_enrolment_freezes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    enrolment_id INTEGER NOT NULL REFERENCES "edtech_enrolments" (id),
    cause TEXT NOT NULL,
    started_on TEXT NOT NULL,
    until_on TEXT,
    counted_days INTEGER NOT NULL DEFAULT 0,
    ended_on TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "edtech_enrolment_freezes_enrolment_id" ON "edtech_enrolment_freezes" (enrolment_id);
