CREATE TABLE "wl_legal_entities" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    trade_licence_no TEXT,
    trade_licence_expires_on TEXT,
    licensing_authority TEXT,
    emirate TEXT,
    trn TEXT,
    vat_registered INTEGER NOT NULL DEFAULT 0,
    address_en TEXT,
    address_ar TEXT,
    footer_en TEXT,
    footer_ar TEXT,
    is_default INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT,
    updated_at TEXT
);

-- At most one default entity.
CREATE UNIQUE INDEX "wl_legal_entities_one_default" ON "wl_legal_entities" (is_default) WHERE is_default = 1;

CREATE TABLE "wl_scope_entities" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_type TEXT NOT NULL,
    scope_id INTEGER NOT NULL,
    legal_entity_id INTEGER NOT NULL REFERENCES "wl_legal_entities" (id),
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (scope_type, scope_id)
);
