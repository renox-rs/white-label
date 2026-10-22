-- The scheduled end of a deployment (09 Manpower §7.1, §4.7, `MPW.RUL.12`, BL-MPW-13): when
-- notice is given to a worker (or by one) the end date of the deployment is capped at the
-- last working day; the end date it replaced is kept (empty if there was none), so
-- that a withdrawn notice can restore it. `exit_announced_at` marks the client and account manager
-- taken care of after the system ended or cancelled the deployment for an exit.
ALTER TABLE "manpower_deployments" ADD COLUMN notice_last_day DATE;
ALTER TABLE "manpower_deployments" ADD COLUMN end_date_before_notice DATE;
ALTER TABLE "manpower_deployments" ADD COLUMN exit_announced_at TIMESTAMPTZ;
