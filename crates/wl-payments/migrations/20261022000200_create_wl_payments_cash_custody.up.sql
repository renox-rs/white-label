-- Cash custody (wl-payments §3 *Cash holder*, *Cash handover*, §4, §5.5,
-- `FIN.CSH.01`; base for #110, #113, #114, #116): the staff who hold cash,
-- the handover of a period, what was collected into it, its counts, the
-- recoveries of a shortage and the postings it leaves. The status of a
-- handover and of a recovery moves only through their machines.
CREATE TABLE "wl_payments_cash_holders" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The staff user who holds cash.
    user_id INTEGER NOT NULL,
    -- `per_person` or `till_session`.
    mode TEXT NOT NULL DEFAULT 'per_person',
    -- `run`, `shift`, `day` or `till_session`.
    period_kind TEXT NOT NULL DEFAULT 'day',
    -- The outlet or branch that bears a written-off loss.
    location TEXT,
    legal_entity_id INTEGER,
    -- The holder's manager; empty takes wl-hr's, else the manager role.
    manager_user_id INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_payments_cash_holders_user" ON "wl_payments_cash_holders" (user_id);

CREATE TABLE "wl_payments_cash_handovers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `period`, or `later_shortage` (raised after the cash was counted).
    kind TEXT NOT NULL DEFAULT 'period',
    holder_id INTEGER NOT NULL REFERENCES "wl_payments_cash_holders" (id),
    holder_user_id INTEGER NOT NULL,
    -- The person recoveries are taken from: the holder, or a till's cashier.
    responsible_user_id INTEGER NOT NULL,
    period_kind TEXT NOT NULL DEFAULT 'day',
    -- The record that is the period (a run), as a subject.
    period_type TEXT,
    period_id INTEGER,
    period_date TEXT NOT NULL,
    -- When the period ends (UTC); empty for a day: 23:59 Asia/Dubai of its date.
    period_end TEXT,
    -- Whether `FIN.RUL.22` escalates it (a vertical rule may replace it).
    escalate INTEGER NOT NULL DEFAULT 1,
    -- Money in fils.
    expected INTEGER NOT NULL DEFAULT 0,
    counted INTEGER NOT NULL DEFAULT 0,
    difference INTEGER NOT NULL DEFAULT 0,
    overage INTEGER NOT NULL DEFAULT 0,
    recovered INTEGER NOT NULL DEFAULT 0,
    written_off INTEGER NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'AED',
    location TEXT,
    legal_entity_id INTEGER,
    -- What caused a later shortage (the reversed delivery).
    source_type TEXT,
    source_id INTEGER,
    reason TEXT,
    counted_by INTEGER,
    counted_at TEXT,
    recounted_by INTEGER,
    -- At a recount: whether the holder was present; `holder_absent` when the
    -- count was made without the holder (`FIN.RUL.22`).
    holder_present INTEGER,
    holder_absent INTEGER NOT NULL DEFAULT 0,
    -- When `FIN.RUL.22` gave the manager the task, and told the admin.
    task_at TEXT,
    admin_notified_at TEXT,
    created_by INTEGER,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cash_handovers_status" ON "wl_payments_cash_handovers" (status);
CREATE INDEX "wl_payments_cash_handovers_holder" ON "wl_payments_cash_handovers" (holder_user_id, period_date);
CREATE INDEX "wl_payments_cash_handovers_period" ON "wl_payments_cash_handovers" (period_type, period_id);
CREATE INDEX "wl_payments_cash_handovers_source" ON "wl_payments_cash_handovers" (source_type, source_id);

-- What was collected into a handover (negative: taken out again).
CREATE TABLE "wl_payments_cash_entries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    handover_id INTEGER NOT NULL REFERENCES "wl_payments_cash_handovers" (id),
    amount INTEGER NOT NULL,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cash_entries_handover" ON "wl_payments_cash_entries" (handover_id);
CREATE INDEX "wl_payments_cash_entries_source" ON "wl_payments_cash_entries" (source_type, source_id);

-- Every count and recount, kept as evidence.
CREATE TABLE "wl_payments_cash_counts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    handover_id INTEGER NOT NULL REFERENCES "wl_payments_cash_handovers" (id),
    -- `count` or `recount`.
    kind TEXT NOT NULL,
    counter_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    holder_present INTEGER,
    holder_absent INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cash_counts_handover" ON "wl_payments_cash_counts" (handover_id);

-- The recoveries of a shortage: cash from the responsible person, or a
-- salary deduction passed to wl-hr (`FIN.ACL.19`).
CREATE TABLE "wl_payments_cash_recoveries" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    handover_id INTEGER NOT NULL REFERENCES "wl_payments_cash_handovers" (id),
    -- The person it is taken from: the handover's responsible person.
    responsible_user_id INTEGER NOT NULL,
    -- `cash` or `salary_deduction`.
    kind TEXT NOT NULL DEFAULT 'cash',
    amount INTEGER NOT NULL,
    note TEXT,
    recorded_by INTEGER,
    -- The approval of a deduction, and who gave it.
    approval_request_id INTEGER,
    approved_by INTEGER,
    -- The holder's acknowledgement of a deduction.
    acknowledged_by INTEGER,
    acknowledged_at TEXT,
    -- The adjustment wl-hr made.
    hr_reference TEXT,
    failure TEXT,
    status TEXT NOT NULL DEFAULT 'recorded',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cash_recoveries_handover" ON "wl_payments_cash_recoveries" (handover_id, status);

-- What a count and a write-off leave for the accountant: an overage held as
-- a liability (not income) and a loss of the holder's branch.
CREATE TABLE "wl_payments_cash_postings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    handover_id INTEGER NOT NULL REFERENCES "wl_payments_cash_handovers" (id),
    -- `overage_liability` or `shortage_loss`.
    kind TEXT NOT NULL,
    amount INTEGER NOT NULL,
    location TEXT,
    legal_entity_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_payments_cash_postings_handover" ON "wl_payments_cash_postings" (handover_id);
