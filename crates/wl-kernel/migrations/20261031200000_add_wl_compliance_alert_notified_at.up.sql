ALTER TABLE "wl_compliance_alerts" ADD COLUMN notified_at TEXT;
-- Alerts raised before the column existed were told when they were stored.
UPDATE "wl_compliance_alerts" SET notified_at = created_at;
