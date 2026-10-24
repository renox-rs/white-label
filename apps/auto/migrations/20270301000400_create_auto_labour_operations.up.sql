-- 08 Auto labour operations catalogue (`AUT.JOB.03`): the menu of the
-- workshop. A flat-rate operation has a flat price for standard hours; an
-- hourly one has an hourly rate (diagnostics, body work).
CREATE TABLE "auto_labour_operations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    pricing TEXT NOT NULL,
    hours_milli INTEGER NOT NULL DEFAULT 0,
    rate INTEGER NOT NULL DEFAULT 0,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_labour_operations_code" ON "auto_labour_operations" (code);
CREATE INDEX "auto_labour_operations_active" ON "auto_labour_operations" (active);
