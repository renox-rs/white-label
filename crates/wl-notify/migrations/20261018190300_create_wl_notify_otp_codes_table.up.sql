-- The one-time codes sent through `send_otp` (wl-notify §6, §7; BL-SAL-23
-- needs them): the state beside Renox's hashed code, so a wrong, expired and
-- voided code can be told apart. The code itself is never stored here.
CREATE TABLE "wl_notify_otp_codes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    purpose TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    channel TEXT NOT NULL DEFAULT 'sms',
    message_id INTEGER,
    expires_at TEXT,
    tries INTEGER NOT NULL DEFAULT 0,
    max_tries INTEGER NOT NULL DEFAULT 3,
    status TEXT NOT NULL DEFAULT 'live',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_notify_otp_codes_subject" ON "wl_notify_otp_codes" (purpose, subject_type, subject_id);
