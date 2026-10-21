-- The completion of a project and the release of its retention (02
-- Construction §4.6, §7.1 *Project* `complete`, *Retention*; BL-CON-07).
-- The handover certificate of the practical completion, the day it is dated
-- and the day the defects liability period ends.
ALTER TABLE "construction_projects" ADD COLUMN handover_certificate TEXT;
ALTER TABLE "construction_projects" ADD COLUMN completed_on TEXT;
ALTER TABLE "construction_projects" ADD COLUMN defects_liability_ends_on TEXT;
-- When the accountant was told that the retention cap was reached.
ALTER TABLE "construction_projects" ADD COLUMN retention_cap_notified_at TEXT;

-- The retention tax invoice of a part, the retention bond linked to it and
-- when the accountant was told it is due.
ALTER TABLE "construction_retentions" ADD COLUMN invoice_id INTEGER;
ALTER TABLE "construction_retentions" ADD COLUMN bond_id INTEGER;
ALTER TABLE "construction_retentions" ADD COLUMN announced_at TEXT;

-- The notice that VAT on the retention becomes due (`CON.RUL.07`): one per
-- project, which the accountant may dismiss with a reason.
CREATE TABLE "construction_retention_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- The day twelve months after the completion certificate, or after the
    -- termination effective date, ends.
    limit_on TEXT NOT NULL,
    notified_on TEXT NOT NULL,
    dismissed_by INTEGER,
    dismissed_reason TEXT,
    dismissed_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "construction_retention_notices_project" ON "construction_retention_notices" (project_id);
