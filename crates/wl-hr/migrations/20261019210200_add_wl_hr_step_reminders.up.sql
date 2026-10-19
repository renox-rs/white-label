-- Reminders of overdue onboarding steps (wl-hr §6 `HRM.RUL.17`, issue #218):
-- the day the assignees were last reminded (every R days from it), and the
-- day HR and the manager were told (once).
ALTER TABLE "wl_hr_onboarding_steps" ADD COLUMN last_reminded_on TEXT;
ALTER TABLE "wl_hr_onboarding_steps" ADD COLUMN escalated_on TEXT;
