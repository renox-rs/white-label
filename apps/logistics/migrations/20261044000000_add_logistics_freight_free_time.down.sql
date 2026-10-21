ALTER TABLE "logistics_job_lines" DROP COLUMN liability_reason;
ALTER TABLE "logistics_job_lines" DROP COLUMN billed_days;
ALTER TABLE "logistics_job_lines" DROP COLUMN charge_type;
ALTER TABLE "logistics_job_lines" DROP COLUMN container_id;
DROP TABLE "logistics_freight_free_time_notices";
DROP TABLE "logistics_freight_free_time_log";
ALTER TABLE "logistics_freight_containers" DROP COLUMN returned_empty_on;
ALTER TABLE "logistics_freight_containers" DROP COLUMN collected_on;
