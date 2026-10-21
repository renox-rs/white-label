DROP TABLE "manpower_deployment_suspensions";
ALTER TABLE "manpower_deployments" DROP COLUMN suspension_announced_at;
ALTER TABLE "manpower_deployments" DROP COLUMN suspended_days;
ALTER TABLE "manpower_deployments" DROP COLUMN resumed_on;
ALTER TABLE "manpower_deployments" DROP COLUMN suspended_on;
