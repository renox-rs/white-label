DROP TABLE IF EXISTS "wl_attendance_adjustment_evidence";
ALTER TABLE "wl_attendance_timesheets" DROP COLUMN dispute_upheld_at;
ALTER TABLE "wl_attendance_timesheets" DROP COLUMN credit_note_id;
