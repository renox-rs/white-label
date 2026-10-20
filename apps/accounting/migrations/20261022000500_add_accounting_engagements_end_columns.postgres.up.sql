-- 04 Accounting §7.1 *Engagement* `end` (BL-ACC-06): the end date scheduled for
-- later, and the partner's confirmation that there is nothing left to bill.
ALTER TABLE "accounting_engagements" ADD COLUMN end_scheduled_on DATE;
ALTER TABLE "accounting_engagements" ADD COLUMN end_scheduled_reason TEXT;
ALTER TABLE "accounting_engagements" ADD COLUMN end_scheduled_by BIGINT;
ALTER TABLE "accounting_engagements" ADD COLUMN nothing_to_bill_on DATE;
ALTER TABLE "accounting_engagements" ADD COLUMN nothing_to_bill_by BIGINT;
