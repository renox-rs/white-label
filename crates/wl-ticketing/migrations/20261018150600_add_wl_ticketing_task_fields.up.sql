-- Staff tasks (wl-notify give-ups, #56): a ticket may carry its own due time
-- and a key its opener finds it by while it is open, and tickets get
-- comments (the Ticket comment entity of wl-ticketing §3).

ALTER TABLE "wl_ticketing_tickets" ADD COLUMN due_at TEXT;
ALTER TABLE "wl_ticketing_tickets" ADD COLUMN source_key TEXT;

CREATE INDEX "wl_ticketing_tickets_source_key" ON "wl_ticketing_tickets" (source_key);

-- A comment on a ticket. No author means the system wrote it; `internal`
-- comments are for staff only, `requester` ones the requester sees too.
CREATE TABLE "wl_ticketing_comments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL REFERENCES "wl_ticketing_tickets" (id) ON DELETE CASCADE,
    author_user_id INTEGER,
    visibility TEXT NOT NULL DEFAULT 'internal',
    text TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_ticketing_comments_ticket" ON "wl_ticketing_comments" (ticket_id);
