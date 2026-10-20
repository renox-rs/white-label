-- A reviewer settles the flags of a record when they accept, correct or
-- reject it (wl-attendance §4, `HRM.ACL.09`, issue #182). The flag stays as
-- history.
ALTER TABLE "wl_attendance_flags" ADD COLUMN resolved_at TEXT;
ALTER TABLE "wl_attendance_flags" ADD COLUMN resolved_by INTEGER;
ALTER TABLE "wl_attendance_flags" ADD COLUMN resolution TEXT;
