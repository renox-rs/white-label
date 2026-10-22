-- The leave request lifecycle (wl-hr §4 *Leave request*, BL-HRM-02 base): who
-- asked and who decided, the unpaid part, and the sick note that may follow
-- a same-day `record_sick`.
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN unpaid_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN requested_by INTEGER;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN decided_by INTEGER;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN decided_at TEXT;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN decision_reason TEXT;
-- True when the request was recorded as approved at once (`record_sick`).
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN recorded_sick BOOLEAN NOT NULL DEFAULT FALSE;
-- The number of the stored sick note and the day it was attached.
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN sick_note_document_id INTEGER;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN sick_note_attached_on TEXT;
-- The last day the note may follow (23:59 Asia/Dubai); the HR task is raised
-- at 00:00 the next day and only once.
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN sick_note_due_on TEXT;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN sick_note_task_id INTEGER;
ALTER TABLE "wl_hr_leave_requests" ADD COLUMN sick_note_task_raised_on TEXT;
