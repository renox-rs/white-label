ALTER TABLE "wl_payments_followup_tasks" DROP COLUMN evidence;
ALTER TABLE "wl_payments_followup_tasks" DROP COLUMN assignee_id;
ALTER TABLE "wl_payments_followup_tasks" DROP COLUMN cheque_id;
DROP TABLE IF EXISTS "wl_payments_cheque_returns";
DROP TABLE IF EXISTS "wl_payments_cheque_links";
DROP TABLE IF EXISTS "wl_payments_cheques";
