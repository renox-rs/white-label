DROP TABLE "wl_integration_devices";
ALTER TABLE "wl_integration_connections" DROP COLUMN grouped;
ALTER TABLE "wl_integration_connections" DROP COLUMN window_end;
ALTER TABLE "wl_integration_connections" DROP COLUMN window_start;
ALTER TABLE "wl_integration_connections" DROP COLUMN heartbeat_seconds;
ALTER TABLE "wl_integration_connections" DROP COLUMN scope;
ALTER TABLE "wl_integration_connections" DROP COLUMN kind;
