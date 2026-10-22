-- Resources, service types and availability (wl-scheduling §3, SCH.RES.01).
CREATE TABLE "wl_scheduling_resources" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    -- person, room, bay, equipment or online.
    resource_type TEXT NOT NULL DEFAULT 'person',
    -- The branch, workshop or campus (a Subject), if any.
    location_type TEXT,
    location_id INTEGER,
    -- The employee (wl-hr) behind a person, or the asset (wl-inventory).
    employee_id INTEGER,
    asset_id INTEGER,
    -- 1, or N for a group slot.
    capacity INTEGER NOT NULL DEFAULT 1,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_scheduling_resources_active" ON "wl_scheduling_resources" (active);

CREATE TABLE "wl_scheduling_service_types" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    duration_minutes INTEGER NOT NULL DEFAULT 30,
    buffer_before_minutes INTEGER NOT NULL DEFAULT 0,
    buffer_after_minutes INTEGER NOT NULL DEFAULT 0,
    -- Seats per slot.
    capacity INTEGER NOT NULL DEFAULT 1,
    -- The resource types one booking needs, one of each: `person,room`.
    required_types TEXT NOT NULL DEFAULT 'person',
    requires_acceptance BOOLEAN NOT NULL DEFAULT FALSE,
    self_bookable BOOLEAN NOT NULL DEFAULT TRUE,
    online_allowed BOOLEAN NOT NULL DEFAULT FALSE,
    check_in_required BOOLEAN NOT NULL DEFAULT FALSE,
    track_in_progress BOOLEAN NOT NULL DEFAULT FALSE,
    cancel_cutoff_hours INTEGER NOT NULL DEFAULT 24,
    reschedule_cutoff_hours INTEGER NOT NULL DEFAULT 24,
    -- The booking deposit in fils; 0 = none. Forfeited on a no-show or on a
    -- cancellation less than `deposit_forfeit_hours` before the start.
    deposit_amount INTEGER NOT NULL DEFAULT 0,
    deposit_forfeit_hours INTEGER NOT NULL DEFAULT 24,
    -- After check-in: the remainder of the deposit is forfeited (else refunded).
    walk_out_forfeits BOOLEAN NOT NULL DEFAULT TRUE,
    -- The prepayment of a self-booking in fils; 0 = none.
    prepayment_amount INTEGER NOT NULL DEFAULT 0,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_scheduling_service_types_code" ON "wl_scheduling_service_types" (code);

-- Which resources can deliver a service type.
CREATE TABLE "wl_scheduling_service_resources" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    service_type_id INTEGER NOT NULL REFERENCES "wl_scheduling_service_types" (id),
    resource_id INTEGER NOT NULL REFERENCES "wl_scheduling_resources" (id)
);

CREATE UNIQUE INDEX "wl_scheduling_service_resources_pair" ON "wl_scheduling_service_resources" (service_type_id, resource_id);

-- A recurring working pattern: minutes after local midnight (Asia/Dubai).
CREATE TABLE "wl_scheduling_availability_rules" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    resource_id INTEGER NOT NULL REFERENCES "wl_scheduling_resources" (id),
    -- 1 = Monday .. 7 = Sunday.
    weekday INTEGER NOT NULL,
    from_minute INTEGER NOT NULL,
    to_minute INTEGER NOT NULL,
    -- NULL = every service type of the resource.
    service_type_id INTEGER,
    valid_from TEXT,
    valid_to TEXT,
    created_at TEXT
);

CREATE INDEX "wl_scheduling_availability_rules_resource" ON "wl_scheduling_availability_rules" (resource_id, weekday);

-- Time a resource is not bookable.
CREATE TABLE "wl_scheduling_availability_blocks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    resource_id INTEGER NOT NULL REFERENCES "wl_scheduling_resources" (id),
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    -- manual, leave, roster, session or holiday.
    source TEXT NOT NULL DEFAULT 'manual',
    source_ref TEXT,
    created_at TEXT
);

CREATE INDEX "wl_scheduling_availability_blocks_resource" ON "wl_scheduling_availability_blocks" (resource_id, starts_at);
