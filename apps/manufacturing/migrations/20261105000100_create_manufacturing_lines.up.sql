-- The production line (07 Manufacturing §3.2, §6): the scope supervisors, operators and line tablets are assigned in. The hourly rate (labour and overhead, MFG.CST.01) is in fils; the locations are wl-inventory locations.
CREATE TABLE "manufacturing_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plant_id INTEGER NOT NULL REFERENCES "manufacturing_plants" (id),
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    hourly_rate INTEGER NOT NULL DEFAULT 0,
    store_location_id INTEGER,
    finished_location_id INTEGER,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);
CREATE UNIQUE INDEX "manufacturing_lines_code" ON "manufacturing_lines" (code);
