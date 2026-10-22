-- Recipients no message goes to any more (wl-hr `record_death`): a stopped
-- user or party. A message to one of them is `suppressed` when it is queued.
CREATE TABLE "wl_notify_stopped_recipients" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- `user` or `party`.
    recipient_kind TEXT NOT NULL,
    recipient_id INTEGER NOT NULL,
    -- Why, e.g. `employee_deceased`.
    reason TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_notify_stopped_recipients_who"
    ON "wl_notify_stopped_recipients" (recipient_kind, recipient_id);
