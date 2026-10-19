CREATE TABLE "wl_notify_types" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    module TEXT NOT NULL,
    -- The row of the wl-notify §5.4 default table, e.g. `invoices`.
    group_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT 'transactional',
    urgency TEXT NOT NULL DEFAULT 'normal',
    statutory INTEGER NOT NULL DEFAULT 0,
    -- JSON list of give-up reactions, e.g. ["staff_task"].
    give_up TEXT NOT NULL DEFAULT '[]',
    staff_role TEXT,
    -- JSON list of channels in order, e.g. ["email", "sms"].
    channels TEXT NOT NULL DEFAULT '[]',
    -- JSON list of default texts per channel and language.
    templates TEXT NOT NULL DEFAULT '[]',
    created_at TEXT,
    updated_at TEXT
);
