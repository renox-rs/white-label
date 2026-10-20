-- `FIN.RUL.18` (issue #107): the day the party and the staff were reminded of a
-- bounced cheque, and the day the decision tasks were made. A new bounce (after
-- a re-presentation) is later than both, so the count starts again.
ALTER TABLE "wl_payments_cheques" ADD COLUMN reminded_on TEXT;
ALTER TABLE "wl_payments_cheques" ADD COLUMN decision_task_on TEXT;
