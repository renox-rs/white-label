-- A client site (09 Manpower §6 *Site*): where a worker is deployed. The
-- required licences and passes (`MPW.CMP.01`) come with the issue that builds
-- the guard on them.
CREATE TABLE "manpower_sites" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The client company, a wl-parties party.
    client_party_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    address TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "manpower_sites_client" ON "manpower_sites" (client_party_id);
