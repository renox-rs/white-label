-- The midday break window per scope and year (wl-attendance §3 *Work
-- calendar*, §7; issue #180): outdoor work is prohibited from `starts_minute`
-- to `ends_minute` (minutes after midnight, Asia/Dubai) from `season_from` to
-- `season_to`, set every year from the MoHRE announcement. A row with no scope
-- is the window of every scope that has none of its own. Without a row for
-- the year the default of the module applies (15 June to 15 September,
-- 12:30 to 15:00).
CREATE TABLE "wl_attendance_midday_windows" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_type TEXT,
    scope_id INTEGER,
    season_from TEXT NOT NULL,
    season_to TEXT NOT NULL,
    starts_minute INTEGER NOT NULL,
    ends_minute INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_attendance_midday_windows_scope" ON "wl_attendance_midday_windows" (scope_type, scope_id, season_from);
