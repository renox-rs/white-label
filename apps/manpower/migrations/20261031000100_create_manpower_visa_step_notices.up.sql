-- The notices of a late visa case step (09 Manpower §7.2 `MPW.RUL.14`, BL-MPW-18): the day the PRO
-- was told the step passed its target and the day HR and the MD were told 7 days later. One row
-- per step; a day is empty until that notice is claimed.
CREATE TABLE "manpower_visa_step_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    step_id INTEGER NOT NULL,
    first_notice_on TEXT,
    second_notice_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "manpower_visa_step_notices_step" ON "manpower_visa_step_notices" (step_id);
