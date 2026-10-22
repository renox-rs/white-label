-- A step of a visa case knows its target number of days from the previous
-- mandatory step (or from the case's creation) and whether it is optional
-- (wl-hr §8 `VisaStepTemplate`; the vertical's rule for a late step reads
-- them through the kernel's `VisaSteps`). A step without a target is never late.
ALTER TABLE "wl_hr_visa_case_steps" ADD COLUMN target_days INTEGER;
ALTER TABLE "wl_hr_visa_case_steps" ADD COLUMN optional INTEGER NOT NULL DEFAULT 0;
