-- The licences a site asks of the workers placed there (09 Manpower §6 *Site*,
-- BL-MPW-22): the account manager writes them as words (`SIRA licence, site
-- pass`) so the screen shows what the site needs. The guard on them works from
-- the document kinds (`MPW.ACL.07`), not from this text.
ALTER TABLE "manpower_sites" ADD COLUMN required_licences TEXT;
