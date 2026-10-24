-- 08 Auto: a declined job card (`AUT.RUL.17`): the collect message to the
-- customer gave up, which the advisor sees on the job card.
ALTER TABLE "auto_job_cards" ADD COLUMN collect_message_failed_at TEXT;
