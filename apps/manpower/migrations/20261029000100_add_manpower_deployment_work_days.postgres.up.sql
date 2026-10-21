-- The working days of a deployment (09 Manpower §3.3 `MPW.ACL.15`,
-- BL-MPW-10): the weekdays of its daily shift, `mon,tue,wed,thu,fri,sat`;
-- none means every day. Two deployments of one worker overlap only on a day
-- both work, at hours that intersect.
ALTER TABLE "manpower_deployments" ADD COLUMN work_days TEXT;
