-- 04 Accounting `ACC.ACL.15` (BL-ACC-19): the client's own EmaraTax user when
-- the client filed the return itself, and the tax payable that gets the
-- client its payment reminder.
ALTER TABLE "accounting_returns" ADD COLUMN client_emaratax_user TEXT;
ALTER TABLE "accounting_returns" ADD COLUMN tax_payable BIGINT;
