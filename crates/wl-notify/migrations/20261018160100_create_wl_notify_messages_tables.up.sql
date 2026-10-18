CREATE TABLE "wl_notify_messages" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    type_key TEXT NOT NULL,
    -- The record the message is about (`Subject`).
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    -- The first message of its fallback chain (its own id for the first one).
    chain_id INTEGER,
    -- The message this one falls back from.
    fallback_of INTEGER,
    -- `user`, `party` or `address`.
    recipient_kind TEXT NOT NULL DEFAULT 'party',
    party_id INTEGER,
    user_id INTEGER,
    -- The wl-parties contact point the address came from.
    contact_point_id INTEGER,
    channel TEXT NOT NULL DEFAULT 'email',
    address TEXT,
    language TEXT NOT NULL DEFAULT 'en',
    rendered_subject TEXT,
    rendered_body TEXT NOT NULL DEFAULT '',
    -- JSON template variables of the request, for a fallback's rendering.
    data TEXT NOT NULL DEFAULT '{}',
    provider TEXT,
    provider_message_id TEXT,
    status TEXT NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error_code TEXT,
    last_error TEXT,
    -- `transient`, `permanent` or a hard kind (`hard_bounce`, `invalid_number`, ...).
    error_kind TEXT,
    -- Why it was not sent, e.g. `no_contact_point`, `suppressed`.
    suppress_reason TEXT,
    scheduled_for TEXT,
    sent_at TEXT,
    delivered_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_notify_messages_status" ON "wl_notify_messages" (status);
CREATE INDEX "wl_notify_messages_chain" ON "wl_notify_messages" (chain_id);
CREATE INDEX "wl_notify_messages_subject" ON "wl_notify_messages" (subject_type, subject_id);

CREATE TABLE "wl_notify_suppressions" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Trimmed; lowercased for e-mail.
    address TEXT NOT NULL,
    channel TEXT NOT NULL DEFAULT 'email',
    -- `opt_out`, `hard_bounce`, `complaint` or `invalid`.
    reason TEXT NOT NULL DEFAULT 'hard_bounce',
    -- What put it there, e.g. `message:42`.
    source TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_notify_suppressions_address"
    ON "wl_notify_suppressions" (channel, address);
