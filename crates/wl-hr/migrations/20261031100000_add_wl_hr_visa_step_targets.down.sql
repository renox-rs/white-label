-- Reverts the visa step targets.

ALTER TABLE "wl_hr_visa_case_steps" DROP COLUMN optional;
ALTER TABLE "wl_hr_visa_case_steps" DROP COLUMN target_days;
