-- Access grants (wl-portal §3 *Access grant*, §4, issue #44): what a portal
-- user may do for one party. The status moves only through the AccessGrant
-- machine. Invitations (tokens, expiry mail) and portal roles with their
-- permissions come with their own issues; the roles are kept here as keys.
CREATE TABLE "wl_portal_access_grants" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The portal user (a Renox user); empty while the invitation is open.
    user_id INTEGER,
    -- The person party the portal user is linked to.
    person_party_id INTEGER NOT NULL,
    -- The party the user acts for: themselves, their company, a dependant.
    party_id INTEGER NOT NULL,
    audience TEXT NOT NULL,
    -- Comma-separated portal role keys, e.g. `primary_contact`.
    roles TEXT NOT NULL DEFAULT '',
    -- `invitation`, `self_registration` or `relationship`.
    source TEXT NOT NULL DEFAULT 'invitation',
    valid_from TEXT NOT NULL,
    valid_until TEXT,
    status TEXT NOT NULL DEFAULT 'invited',
    -- The state a suspension came from (`active` or `read_only`), and why.
    suspended_from TEXT,
    suspend_reason TEXT,
    ended_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_portal_access_grants_user" ON "wl_portal_access_grants" (user_id, status);
CREATE INDEX "wl_portal_access_grants_party" ON "wl_portal_access_grants" (party_id, status);
