-- The opening checklist of an outlet for a day (06 F&B §7.2 `FNB.RUL.03`):
-- opened H hours before opening, checked at opening.
CREATE TABLE "fnb_opening_checklists" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    outlet_id INTEGER NOT NULL REFERENCES "fnb_outlets" (id),
    business_date TEXT,
    opened_at TEXT,
    completed_at TEXT,
    notified_at TEXT,
    missing TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "fnb_opening_checklists_day" ON "fnb_opening_checklists" (outlet_id, business_date);
