-- `CON.RUL.08` (02 Construction §4.2, §7.2; BL-CON-15): the absence cases the
-- rule opened for HR, one per absence spell, and whether the project works
-- on Saturdays (the site calendar; Sunday and public holidays never count).
ALTER TABLE "construction_projects" ADD COLUMN works_saturdays INTEGER NOT NULL DEFAULT 0;

CREATE TABLE "construction_absence_cases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- A wl-hr employee id.
    employee_id INTEGER NOT NULL,
    -- The ticket of category `absence` HR works.
    ticket_id INTEGER NOT NULL,
    -- The last day with attendance or approved leave before the spell.
    last_present_on TEXT NOT NULL,
    opened_on TEXT NOT NULL,
    -- The day HR was reminded that `report_absconding` may be requested.
    reminded_on TEXT,
    -- The day the worker was seen again (attendance or leave); the ticket
    -- stays open for HR.
    ended_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_absence_cases_employee" ON "construction_absence_cases" (employee_id);
CREATE INDEX "construction_absence_cases_project" ON "construction_absence_cases" (project_id);
