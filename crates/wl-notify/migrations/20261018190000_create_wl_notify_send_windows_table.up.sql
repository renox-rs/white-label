-- The marketing send window of a channel (NTF.RUL.02, #62 A2): minutes after
-- midnight Asia/Dubai. No row means the default 07:00-21:00.
CREATE TABLE "wl_notify_send_windows" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel TEXT NOT NULL UNIQUE,
    start_minute INTEGER NOT NULL,
    end_minute INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);
