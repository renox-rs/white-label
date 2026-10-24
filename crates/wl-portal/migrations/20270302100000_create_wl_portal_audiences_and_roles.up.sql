-- Audiences and portal roles (wl-portal §3 *Audience*, *Portal role*,
-- BL-POR-02), and the day a grant turned read-only (BL-POR-03, `POR.ACL.01`).
-- A vertical seeds its audiences and their roles; the company admin edits a
-- role's permissions in the portal settings.
CREATE TABLE "wl_portal_audiences" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- e.g. `tenant`, `owner`, `client`, `worker`.
    key TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    -- Days from `end` to `close` (§4 vertical mapping); 0 closes at once.
    close_after_days INTEGER NOT NULL DEFAULT 90,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (key)
);

CREATE TABLE "wl_portal_roles" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    audience TEXT NOT NULL,
    -- e.g. `primary_contact`, `finance_contact`.
    key TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    -- Comma-separated portal permissions, e.g. `portal.invoices.view`.
    permissions TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (audience, key)
);

-- The Asia/Dubai day the grant became `read_only` (`end`): the close-after
-- period counts from the day after it.
ALTER TABLE "wl_portal_access_grants" ADD COLUMN ended_on TEXT;
