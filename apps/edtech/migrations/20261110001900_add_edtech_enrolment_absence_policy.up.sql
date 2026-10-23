-- 10 EdTech absence policy (10 section 4.2 *A student is absent*, section 6
-- *Catalogue item*, BL-EDU-15): the policy of the catalogue item, stored with the
-- enrolment at enrolment time. Whether an absence without a valid notice uses a
-- credit of the package (default yes), and how many hours before a session a
-- notice of absence excuses it (default 24).
ALTER TABLE "edtech_enrolments" ADD COLUMN absence_consumes BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE "edtech_enrolments" ADD COLUMN excuse_notice_hours INTEGER NOT NULL DEFAULT 24;
