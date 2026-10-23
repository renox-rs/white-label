-- 10 EdTech seat hold (`EDU.RUL.11`, 10 section 4.1): the payment link of
-- wl-payments made for the first payment of an enrolment, expiring when the
-- seat hold does. `lapse` cancels it; a failed payment replaces it.
ALTER TABLE "edtech_enrolments" ADD COLUMN payment_link_type TEXT;
ALTER TABLE "edtech_enrolments" ADD COLUMN payment_link_id BIGINT;
ALTER TABLE "edtech_enrolments" ADD COLUMN payment_link_path TEXT;
