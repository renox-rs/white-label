-- The first day an attendance record was saved for the employee (wl-hr §4
-- `cancel_onboarding`, issue #195): the approval of the cancellation re-reads
-- it, so a check-in between the request and the approval is not missed.
ALTER TABLE "wl_hr_employees" ADD COLUMN first_attended_on TEXT;
