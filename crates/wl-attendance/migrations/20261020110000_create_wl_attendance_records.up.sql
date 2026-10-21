-- Work locations and attendance records (wl-attendance §3, §4; issues #178 to
-- #181): the minimal base the checks on attendance run on. The employee is a
-- plain id (the HR module owns the record); the status moves only through the
-- AttendanceRecord machine.
CREATE TABLE "wl_attendance_work_locations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    -- The Subject the location is for (project, branch, client site...).
    scope_type TEXT,
    scope_id INTEGER,
    -- Outdoor work is subject to the midday break (`HRM.RUL.01`).
    outdoor INTEGER NOT NULL DEFAULT 0,
    -- The qualification kinds the context requires (JSON list), checked by
    -- `HRM.RUL.14`.
    required_qualifications TEXT NOT NULL DEFAULT '[]',
    -- The role the context needs, for role-bound qualification kinds.
    qualification_role TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_work_locations_scope" ON "wl_attendance_work_locations" (scope_type, scope_id);

CREATE TABLE "wl_attendance_records" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL,
    location_id INTEGER NOT NULL REFERENCES "wl_attendance_work_locations" (id),
    -- The day in Asia/Dubai.
    date TEXT NOT NULL,
    check_in TEXT NOT NULL,
    check_out TEXT,
    -- `supervisor`, `self_check_in`, `device`, `import`.
    method TEXT NOT NULL DEFAULT 'supervisor',
    recorded_by INTEGER,
    recorded_at TEXT NOT NULL,
    -- Breaks taken (JSON list of from/to local times).
    breaks TEXT NOT NULL DEFAULT '[]',
    note TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'recorded',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_records_employee" ON "wl_attendance_records" (employee_id, date);
CREATE INDEX "wl_attendance_records_location" ON "wl_attendance_records" (location_id, date);

-- A reason a record needs review (wl-attendance §3 *Attendance flag*): one
-- row per record and flag type.
CREATE TABLE "wl_attendance_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    record_id INTEGER NOT NULL REFERENCES "wl_attendance_records" (id) ON DELETE CASCADE,
    flag_type TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (record_id, flag_type)
);
