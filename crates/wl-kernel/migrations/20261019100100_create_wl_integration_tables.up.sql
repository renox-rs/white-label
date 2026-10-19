CREATE TABLE "wl_integration_connections" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL,
    module TEXT NOT NULL,
    owner_role TEXT NOT NULL,
    screens TEXT NOT NULL DEFAULT '[]',
    fallback_en TEXT NOT NULL DEFAULT '',
    fallback_ar TEXT,
    down_after_minutes INTEGER NOT NULL DEFAULT 15,
    status TEXT NOT NULL DEFAULT 'up',
    last_success_at TEXT,
    last_error TEXT,
    failed_since TEXT,
    disabled_reason TEXT,
    status_changed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_integration_connections_key" ON "wl_integration_connections" (key);

CREATE TABLE "wl_integration_queue" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT 'null',
    natural_key TEXT,
    subject_type TEXT,
    subject_id INTEGER,
    status TEXT NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT,
    last_error TEXT,
    enqueued_at TEXT NOT NULL,
    finished_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_integration_queue_connection" ON "wl_integration_queue" (connection_key, status, id);
