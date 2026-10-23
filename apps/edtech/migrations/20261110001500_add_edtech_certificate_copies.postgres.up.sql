-- 10 EdTech certificate copy (10 section 4.4 *A copy is lost*, BL-EDU-16): a
-- lost copy is printed again without any change to the certificate; how many
-- times, when and by whom is kept next to the history row the copy writes.
ALTER TABLE "edtech_certificates" ADD COLUMN copies_printed BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "edtech_certificates" ADD COLUMN last_copy_on DATE;
ALTER TABLE "edtech_certificates" ADD COLUMN last_copy_by BIGINT;
