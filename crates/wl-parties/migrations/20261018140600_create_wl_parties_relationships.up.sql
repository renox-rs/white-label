-- Relationships between parties (wl-parties §3): a directed link from one
-- party to another, e.g. a person who is a finance contact of a company.
CREATE TABLE "wl_parties_relationships" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    to_party_id INTEGER NOT NULL REFERENCES "wl_parties_parties" (id) ON DELETE CASCADE,
    -- `guardian_of`, `contact_of`, `employee_of`, `shareholder_of`,
    -- `family_member_of` or `member_of_group`.
    relation_type TEXT NOT NULL,
    -- For contacts, e.g. `finance`; NULL when there is none.
    position TEXT,
    -- The one the others of its (to party, type, position) give way to.
    is_primary INTEGER NOT NULL DEFAULT 0,
    valid_from TEXT NOT NULL,
    -- NULL while the relationship has no end.
    valid_until TEXT,
    end_reason TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_parties_relationships_to" ON "wl_parties_relationships" (to_party_id, relation_type);
CREATE INDEX "wl_parties_relationships_from" ON "wl_parties_relationships" (from_party_id);

-- At most one primary per (to party, type, position).
CREATE UNIQUE INDEX "wl_parties_relationships_one_primary"
    ON "wl_parties_relationships" (to_party_id, relation_type, COALESCE(position, ''))
    WHERE is_primary = 1;
