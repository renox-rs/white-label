-- What an end records (09 Manpower §7.1 *Deployment* `end`, §4.1, BL-MPW-14): the client's note
-- of a rejection, the waiver the MD attached to a short notice, and the reason a
-- scheduled end will be made with (client non-payment: the worker stays until the
-- end date the notice period sets, then the daily run ends the deployment with this
-- reason instead of *end date*).
ALTER TABLE "manpower_deployments" ADD COLUMN end_note TEXT;
ALTER TABLE "manpower_deployments" ADD COLUMN end_waiver TEXT;
ALTER TABLE "manpower_deployments" ADD COLUMN planned_end_reason TEXT;
