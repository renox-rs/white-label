-- `CON.RUL.10` (02 Construction §4.5, §7.2; BL-CON-14): the days after a delay
-- event within which the contract requires notice (none by default), and the
-- days the project manager and QS were warned for a draft variation.
ALTER TABLE "construction_projects" ADD COLUMN notice_days INTEGER;
ALTER TABLE "construction_variations" ADD COLUMN notice_warned_7_on TEXT;
ALTER TABLE "construction_variations" ADD COLUMN notice_warned_2_on TEXT;
