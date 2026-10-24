-- 08 Auto: vehicles that are not collected (`AUT.RUL.18`, BL-AUT-05). The daily
-- run remembers the days of its reminders (the first one starts the storage
-- fee), the day-30 task for the branch manager, and the storage fee invoice
-- raised at release; the client's switches (the storage fee is off until it is
-- turned on) are rows of `auto_settings`.
ALTER TABLE "auto_job_cards" ADD COLUMN collect_first_on TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN collect_second_on TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN collect_manager_on TEXT;
ALTER TABLE "auto_job_cards" ADD COLUMN storage_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "auto_job_cards" ADD COLUMN storage_invoice_id INTEGER;
ALTER TABLE "auto_job_cards" ADD COLUMN storage_invoiced_at TEXT;

CREATE TABLE "auto_settings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_settings_key" ON "auto_settings" (key);
