-- Ticketing configuration (wl-ticketing §3, §7): SLA policies with their
-- targets per priority, hold reasons and categories. Seeded idempotently with
-- the §7 defaults and changed by the client afterwards.

-- An SLA policy; `standard` is the seeded default.
CREATE TABLE "wl_ticketing_sla_policies" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

-- The targets of a policy for one priority, in working minutes.
CREATE TABLE "wl_ticketing_sla_targets" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    policy_id INTEGER NOT NULL REFERENCES "wl_ticketing_sla_policies" (id) ON DELETE CASCADE,
    priority TEXT NOT NULL,
    response_minutes INTEGER NOT NULL,
    resolution_minutes INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (policy_id, priority)
);

-- Why a ticket may be put on hold; a pausing reason stops the SLA clock.
CREATE TABLE "wl_ticketing_hold_reasons" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    pauses_sla INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

-- A ticket category: the default priority and SLA policy, and whether it
-- needs the requester's sign-off, is confidential or chargeable.
CREATE TABLE "wl_ticketing_categories" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    default_priority TEXT NOT NULL DEFAULT 'normal',
    sla_policy_id INTEGER REFERENCES "wl_ticketing_sla_policies" (id) ON DELETE SET NULL,
    requires_signoff INTEGER NOT NULL DEFAULT 0,
    confidential INTEGER NOT NULL DEFAULT 0,
    chargeable INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);
