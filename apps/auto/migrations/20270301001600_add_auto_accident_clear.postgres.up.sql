-- 08 Auto: clearing the accident damage flag of a job card needs a reason and
-- the workshop manager's approval, never the requester's (`AUT.ACL.20`,
-- BL-AUT-07). The request waits on the job card until it is decided.
ALTER TABLE "auto_job_cards" ADD COLUMN accident_clear_reason TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN accident_clear_by BIGINT;
ALTER TABLE "auto_job_cards" ADD COLUMN accident_clear_at TIMESTAMPTZ;
