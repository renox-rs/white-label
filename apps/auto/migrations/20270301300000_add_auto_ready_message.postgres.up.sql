-- The vehicle-ready SMS (AUT.RUL.02) gave up: the job card shows "message not
-- delivered" to the advisor and no task is made (08 §7.1 Notification types).
ALTER TABLE "auto_job_cards" ADD COLUMN ready_message_failed_at TIMESTAMPTZ;
