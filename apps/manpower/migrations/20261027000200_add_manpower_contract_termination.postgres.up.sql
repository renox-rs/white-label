-- The early termination of a client contract (09 Manpower §7.1 *Client
-- contract* `terminate`, `MPW.ACL.17`, BL-MPW-04): who ended it, why, the
-- notice and the end date it had before, and the day the client was sent
-- the confirmation.
ALTER TABLE "manpower_client_contracts" ADD COLUMN original_end_date DATE;
-- `client` or `agency`.
ALTER TABLE "manpower_client_contracts" ADD COLUMN termination_initiated_by TEXT;
ALTER TABLE "manpower_client_contracts" ADD COLUMN termination_reason TEXT;
ALTER TABLE "manpower_client_contracts" ADD COLUMN termination_notice_on DATE;
-- The waiver document of a notice shorter than the contract's notice period.
ALTER TABLE "manpower_client_contracts" ADD COLUMN termination_waiver TEXT;
ALTER TABLE "manpower_client_contracts" ADD COLUMN termination_requested_by BIGINT;
-- When the MD's approval made it `terminating`.
ALTER TABLE "manpower_client_contracts" ADD COLUMN terminated_at TIMESTAMPTZ;
-- When the client was sent the confirmation and the deployments cut were announced.
ALTER TABLE "manpower_client_contracts" ADD COLUMN termination_announced_at TIMESTAMPTZ;
