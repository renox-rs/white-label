-- The escalation of a driver who does not hand over the cash of a run (03
-- Logistics §7.2 `LOG.RUL.13`, which replaces `FIN.RUL.22` for drivers;
-- BL-LOG-13): one row per cash handover the rule acted on, so each step is
-- taken once.
CREATE TABLE "logistics_cash_watches" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The wl-payments cash handover of the run.
    handover_id INTEGER NOT NULL,
    -- The run (the handover's period record) and its day.
    run_id INTEGER NOT NULL,
    run_date TEXT NOT NULL,
    -- The driver, a user.
    driver_user_id INTEGER NOT NULL,
    -- The clock start: the run's return to the depot, else the end of the
    -- driver's shift, else 23:59 of the run date.
    started_at TEXT NOT NULL,
    -- Whether the run was still out when the first step was taken.
    run_out INTEGER NOT NULL DEFAULT 0,
    -- N hours: the operations manager's task to call the driver.
    task_at TEXT,
    task_ticket_id INTEGER,
    -- M hours: the company admin's notice and the manager's request to count.
    admin_at TEXT,
    count_ticket_id INTEGER,
    -- The absence note on the driver's HR record, written once at M hours,
    -- or why it was not (`checked_in`, `no_employee`).
    absence_noted_at TEXT,
    absence_skipped TEXT,
    -- When the handover left `open` and the tasks were closed.
    closed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "logistics_cash_watches_handover" ON "logistics_cash_watches" (handover_id);
