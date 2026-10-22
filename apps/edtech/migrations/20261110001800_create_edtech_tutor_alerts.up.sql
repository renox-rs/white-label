-- 10 EdTech tutor alert (10 section 7.2 `EDU.RUL.12`, BL-EDU-10): one row per
-- tutor and trigger (a document expiring, a notice given, a manual availability
-- block, a leave). It names the first day the tutor cannot teach, the last day
-- for a block or a leave, and the key of the one task the coordinator has for
-- it. The sessions it flags carry its id in `flag_alert_id`.
CREATE TABLE "edtech_tutor_alerts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL,
    trigger_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    document_kind TEXT,
    from_date TEXT NOT NULL,
    until_date TEXT,
    starts_at TEXT,
    ends_at TEXT,
    detail TEXT,
    task_key TEXT NOT NULL,
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_tutor_alerts_employee_id_trigger_key" ON "edtech_tutor_alerts" (employee_id, trigger_key);
CREATE INDEX "edtech_tutor_alerts_employee_id" ON "edtech_tutor_alerts" (employee_id);
