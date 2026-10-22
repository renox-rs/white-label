-- 10 EdTech freeze (10 section 7.1 *Enrolment*, `freeze`/`unfreeze`,
-- BL-EDU-04, BL-EDU-05): the day the enrolment became `frozen`, whatever the
-- causes that held it since. The package is given back the days from then to
-- the day the last cause ended.
ALTER TABLE "edtech_enrolments" ADD COLUMN frozen_on DATE;
