-- The plant (07 Manufacturing §3.2): the scope production managers and QC inspectors are assigned in.
CREATE TABLE "manufacturing_plants" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);
