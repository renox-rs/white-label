-- One row per confirmed agent action (BL-AGT-08): the confirmation token's
-- nonce (one run per token), who ran it through which assistant, and the
-- result a repeated confirm returns.
CREATE TABLE "wl_agent_runs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nonce TEXT NOT NULL,
    action TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id INTEGER,
    user_id INTEGER NOT NULL,
    client TEXT NOT NULL,
    outcome TEXT NOT NULL,
    result TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_agent_runs_nonce" ON "wl_agent_runs" (nonce);
CREATE INDEX "wl_agent_runs_user" ON "wl_agent_runs" (user_id);
