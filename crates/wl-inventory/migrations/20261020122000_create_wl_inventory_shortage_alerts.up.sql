-- Where stock stays short (wl-inventory §6 `STK.RUL.06`): one row per item
-- and location while a pending issue is open or the balance is negative,
-- kept to tell the storekeeper at most once an hour and to escalate once.
CREATE TABLE "wl_inventory_shortage_alerts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES "wl_inventory_items" (id),
    location_id INTEGER NOT NULL REFERENCES "wl_inventory_locations" (id),
    -- When it first became short.
    first_short_at TEXT NOT NULL,
    -- When the storekeeper was last told.
    last_notified_at TEXT,
    -- When the task was opened and the manager told.
    escalated_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "wl_inventory_shortage_alerts_place"
    ON "wl_inventory_shortage_alerts" (item_id, location_id);
