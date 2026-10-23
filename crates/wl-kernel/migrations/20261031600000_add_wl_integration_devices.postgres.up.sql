-- Devices at a site that call the platform (BL-KRN-07).
ALTER TABLE "wl_integration_connections" ADD COLUMN kind TEXT NOT NULL DEFAULT 'service';
ALTER TABLE "wl_integration_connections" ADD COLUMN scope TEXT;
ALTER TABLE "wl_integration_connections" ADD COLUMN heartbeat_seconds BIGINT NOT NULL DEFAULT 60;
ALTER TABLE "wl_integration_connections" ADD COLUMN window_start BIGINT;
ALTER TABLE "wl_integration_connections" ADD COLUMN window_end BIGINT;
ALTER TABLE "wl_integration_connections" ADD COLUMN grouped BOOLEAN NOT NULL DEFAULT FALSE;

CREATE TABLE "wl_integration_devices" (
    id BIGSERIAL PRIMARY KEY,
    connection_key TEXT NOT NULL,
    device_key TEXT NOT NULL,
    last_heartbeat_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX "wl_integration_devices_device" ON "wl_integration_devices" (connection_key, device_key);
