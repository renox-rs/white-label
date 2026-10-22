-- The end of a client contract (09 Manpower §7.1 *Client contract* `end`,
-- BL-MPW-06): when the system ended it, and when the deployments, the
-- timesheets and the account manager were taken care of after the commit.
ALTER TABLE "manpower_client_contracts" ADD COLUMN ended_at TIMESTAMPTZ;
ALTER TABLE "manpower_client_contracts" ADD COLUMN end_announced_at TIMESTAMPTZ;
