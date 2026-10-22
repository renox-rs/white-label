-- Devices at a site that call the platform (BL-KRN-07, wl-kernel §4.5
-- *Connection kinds*): a connection is a `service` the platform calls or a
-- `device` whose health signal is its heartbeat. A device connection has a
-- heartbeat interval (seconds), an active window (minutes after midnight in
-- Asia/Dubai; NULL: always), a scope (outlet, branch, site) and may group the
-- devices of the scope.
ALTER TABLE "wl_integration_connections" ADD COLUMN kind TEXT NOT NULL DEFAULT 'service';
ALTER TABLE "wl_integration_connections" ADD COLUMN scope TEXT;
ALTER TABLE "wl_integration_connections" ADD COLUMN heartbeat_seconds INTEGER NOT NULL DEFAULT 60;
ALTER TABLE "wl_integration_connections" ADD COLUMN window_start INTEGER;
ALTER TABLE "wl_integration_connections" ADD COLUMN window_end INTEGER;
ALTER TABLE "wl_integration_connections" ADD COLUMN grouped INTEGER NOT NULL DEFAULT 0;

-- One row per device of a device connection: when it last sent a heartbeat.
CREATE TABLE "wl_integration_devices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_key TEXT NOT NULL,
    device_key TEXT NOT NULL,
    last_heartbeat_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_integration_devices_device" ON "wl_integration_devices" (connection_key, device_key);
