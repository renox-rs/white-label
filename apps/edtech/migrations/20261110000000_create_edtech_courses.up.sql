-- 10 EdTech course (`EDU.CRS.01`, `EDU.CRS.02`): what is taught, with the
-- regulator approval recorded where the emirate asks for one.
CREATE TABLE "edtech_courses" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    name_en TEXT NOT NULL,
    name_ar TEXT,
    programme TEXT NOT NULL DEFAULT '',
    level TEXT,
    modules TEXT,
    hours INTEGER NOT NULL DEFAULT 0,
    prerequisites TEXT,
    delivery TEXT NOT NULL,
    for_minors BOOLEAN NOT NULL DEFAULT FALSE,
    approval_required BOOLEAN NOT NULL DEFAULT FALSE,
    approval_no TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_courses_code" ON "edtech_courses" (code);
CREATE INDEX "edtech_courses_active" ON "edtech_courses" (active);
