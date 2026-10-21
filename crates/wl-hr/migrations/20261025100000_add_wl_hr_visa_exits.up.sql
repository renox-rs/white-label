-- The visa case as far as the refusal and the non-standard exits need it
-- (wl-hr §3 *Visa case*, §4, issues #193, #195, #196, #198).

-- A renewal case knows the residence visa it renews: the follow-up task after
-- a refusal is due N days before this date.
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN current_expiry TEXT;

-- The refusal: the authority's reference, the reason, the day, and the day the
-- follow-up task is due (renewal only).
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN rejection_reference TEXT;
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN rejection_reason TEXT;
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN rejected_on TEXT;
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN deadline TEXT;

-- *Cancellation not needed*, on a *new* case whose permit was issued: by whom,
-- why, when.
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN cancellation_not_needed_by INTEGER;
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN cancellation_not_needed_reason TEXT;
ALTER TABLE "wl_hr_visa_cases" ADD COLUMN cancellation_not_needed_on TEXT;

-- One step of a case. A work permit or entry permit step that is done means
-- the permit was issued.
CREATE TABLE "wl_hr_visa_case_steps" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    case_id INTEGER NOT NULL REFERENCES "wl_hr_visa_cases" (id) ON DELETE CASCADE,
    -- `work_permit`, `entry_permit`, `medical_fitness`, ...
    step_type TEXT NOT NULL,
    -- The day it was done; empty while open.
    done_on TEXT,
    reference TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_visa_case_steps_case" ON "wl_hr_visa_case_steps" (case_id);

-- What an exit leaves on the employee: the last working day (cleared by
-- `withdraw_notice` and `reinstate`), the exit route (`onboarding_cancelled`,
-- `absconding`, ...) and when the portal account was closed.
ALTER TABLE "wl_hr_employees" ADD COLUMN last_working_day TEXT;
ALTER TABLE "wl_hr_employees" ADD COLUMN exit_route TEXT;
ALTER TABLE "wl_hr_employees" ADD COLUMN portal_closed_at TEXT;
