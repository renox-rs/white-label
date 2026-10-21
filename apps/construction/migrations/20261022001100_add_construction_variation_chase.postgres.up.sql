-- `CON.RUL.09` (02 Construction §7.2; BL-CON-13): the day the project manager
-- was reminded to chase the client and the day the MD was notified, for the
-- latest submission of the variation. `submit` clears both.
ALTER TABLE "construction_variations" ADD COLUMN chased_on DATE;
ALTER TABLE "construction_variations" ADD COLUMN escalated_on DATE;
