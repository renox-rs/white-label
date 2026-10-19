-- One statutory notice that gave up, with the proof staff recorded for it
-- (#59 A3). One row per given-up chain.
CREATE TABLE "wl_notify_statutory_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The first message of the chain that gave up; a retried run finds the row.
    chain_id INTEGER NOT NULL,
    type_key TEXT NOT NULL,
    -- The record the notice is about (`Subject`).
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    subject_label TEXT,
    -- JSON of the kernel's recipient.
    recipient TEXT NOT NULL DEFAULT '{}',
    -- The message that gave up.
    message_id INTEGER NOT NULL,
    gave_up_at TEXT NOT NULL,
    -- The staff task, once opened.
    ticket_id INTEGER,
    -- The proof: none until staff record it.
    proof_method TEXT,
    delivered_on TEXT,
    proof_note TEXT,
    recorded_by INTEGER,
    recorded_at TEXT,
    attachment_key TEXT,
    attachment_name TEXT,
    attachment_mime TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_notify_statutory_notices_chain"
    ON "wl_notify_statutory_notices" (chain_id);
CREATE INDEX "wl_notify_statutory_notices_subject"
    ON "wl_notify_statutory_notices" (subject_type, subject_id);
