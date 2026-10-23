-- The dispatches a recall stopped (BL-MFG-10): one row per recall and dispatch
-- that has not left and holds a recalled lot, with the storekeeper's task.
CREATE TABLE "manufacturing_repick_flags" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recall_id INTEGER NOT NULL,
    dispatch_id INTEGER NOT NULL,
    number TEXT NOT NULL,
    ticket_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_repick_flags_pair" ON "manufacturing_repick_flags" (recall_id, dispatch_id);
