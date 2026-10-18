CREATE TABLE "wl_parties_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    legal_name_en TEXT NOT NULL,
    legal_name_ar TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    default_language TEXT NOT NULL DEFAULT 'en',
    preferred_channel TEXT NOT NULL DEFAULT 'email',
    -- The display block reason only (`on_hold` for a restricted one); NULL while not blocked.
    hold_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_parties_status" ON "wl_parties_parties" (status);

CREATE TABLE "wl_parties_person_details" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL UNIQUE REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    date_of_birth TEXT,
    nationality TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "wl_parties_company_details" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL UNIQUE REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    legal_form TEXT,
    licence_authority TEXT,
    trade_licence_no TEXT,
    vat_registered INTEGER NOT NULL DEFAULT 0,
    trn TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "wl_parties_roles" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    role_key TEXT NOT NULL,
    since TEXT,
    until TEXT,
    external_reference TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_roles_party" ON "wl_parties_roles" (party_id, role_key);

CREATE TABLE "wl_parties_contact_points" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    contact_type TEXT NOT NULL,
    value TEXT NOT NULL,
    label TEXT,
    is_primary INTEGER NOT NULL DEFAULT 0,
    verified_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_contact_points_party" ON "wl_parties_contact_points" (party_id);

-- At most one primary contact point per type and party.
CREATE UNIQUE INDEX "wl_parties_contact_points_one_primary"
    ON "wl_parties_contact_points" (party_id, contact_type) WHERE is_primary = 1;
