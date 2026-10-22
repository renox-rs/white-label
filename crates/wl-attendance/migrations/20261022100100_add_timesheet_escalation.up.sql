-- When the approver of a timesheet was escalated to (wl-attendance §5.6,
-- `HRM.RUL.15` at E days; issue #187). Set once per `submit`; a new `submit`
-- clears it, so that the clock restarts.
ALTER TABLE "wl_attendance_timesheets" ADD COLUMN escalated_at TEXT;
