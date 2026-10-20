-- 04 Accounting `ACC.ACL.04` (BL-ACC-18): the correction of a filed return:
-- the error, the day it was found, the later return the adjustment goes into
-- (or its period while it does not exist) and the voluntary disclosure task.
ALTER TABLE "accounting_returns" ADD COLUMN error_amount INTEGER;
ALTER TABLE "accounting_returns" ADD COLUMN error_found_on TEXT;
ALTER TABLE "accounting_returns" ADD COLUMN correction_return_id INTEGER;
ALTER TABLE "accounting_returns" ADD COLUMN correction_period_end TEXT;
ALTER TABLE "accounting_returns" ADD COLUMN disclosure_task_id INTEGER;
