-- 10 EdTech `withdraw` (10 section 7.1 *Enrolment*, BL-EDU-03): what the
-- withdrawal has done about the money so far (membership cancelled, link
-- cancelled, invoices closed, credit notes made, refunds asked, the invoice
-- for the amount owed), so a step that failed is repeated without the others.
ALTER TABLE "edtech_enrolments" ADD COLUMN settlement TEXT NOT NULL DEFAULT '{}';
