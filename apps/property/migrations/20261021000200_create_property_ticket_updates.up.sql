-- Field updates of a ticket entered after the fact (01 Property §7.2, row 16,
-- `PLT.ACL.10`, BL-PRP-27): the work time is the business time; an update
-- entered more than 30 minutes late carries the late-entry flag and its reason.
CREATE TABLE "property_ticket_updates" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL,
    note TEXT NOT NULL,
    worked_at TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    late BOOLEAN NOT NULL DEFAULT FALSE,
    late_reason TEXT,
    late_entry_id INTEGER,
    entered_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "property_ticket_updates_ticket" ON "property_ticket_updates" (ticket_id);
