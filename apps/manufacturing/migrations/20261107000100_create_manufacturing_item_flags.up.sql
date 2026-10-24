-- The conformity flags of an item (BL-MFG-11, MFG.ACL.11): regulated (needs a
-- conformity certificate to leave the plant), export only (single-use
-- plastics) and the certificate kinds that count; empty: all three.
CREATE TABLE "manufacturing_item_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL,
    regulated INTEGER NOT NULL DEFAULT 0,
    export_only INTEGER NOT NULL DEFAULT 0,
    accepted_kinds TEXT NOT NULL DEFAULT '',
    set_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_item_flags_item" ON "manufacturing_item_flags" (item_id);
