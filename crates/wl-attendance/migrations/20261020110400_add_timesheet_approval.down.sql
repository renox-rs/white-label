DROP TABLE IF EXISTS "wl_attendance_approval_evidence";
ALTER TABLE "wl_attendance_timesheets" DROP COLUMN approved_at;
ALTER TABLE "wl_attendance_timesheets" DROP COLUMN approved_by;
