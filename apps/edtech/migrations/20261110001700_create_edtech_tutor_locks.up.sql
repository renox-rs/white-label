-- 10 EdTech tutor calendar lock (10 section 7.1 *Batch* `change_tutor`,
-- BL-EDU-09): one row per tutor (a wl-scheduling resource). A change of tutor
-- writes the new tutor's row first, so two changes that give the same tutor
-- sessions at the same time wait for each other: the second one checks the
-- calendar after the first committed.
CREATE TABLE "edtech_tutor_locks" (
    resource_id INTEGER PRIMARY KEY,
    locked_at TEXT NOT NULL
);
