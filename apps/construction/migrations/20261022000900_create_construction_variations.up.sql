-- The Variation (02 Construction §4.5, §6, §7.1 *Variation*; BL-CON-11).
-- Money is fils; an omission is stored negative.
-- The contract completion date: an approved extension of time moves it.
ALTER TABLE "construction_projects" ADD COLUMN completion_due_on TEXT;

CREATE TABLE "construction_variations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- `VO-07`; every revision of the variation has the same code.
    code TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    -- `addition`, `omission` or `time`.
    kind TEXT NOT NULL DEFAULT 'addition',
    description TEXT NOT NULL DEFAULT '',
    -- The client's or consultant's instruction, and whether the work was
    -- done before the client approved.
    site_instruction TEXT,
    executed_before_approval INTEGER NOT NULL DEFAULT 0,
    -- The variation's own cost code.
    cost_code TEXT NOT NULL,
    priced_amount INTEGER NOT NULL DEFAULT 0,
    approved_amount INTEGER,
    -- The cost booked on the cost code, and the part of it that nothing
    -- recovers.
    cost_booked INTEGER NOT NULL DEFAULT 0,
    unrecovered_amount INTEGER NOT NULL DEFAULT 0,
    client_reference TEXT,
    rejection_reason TEXT,
    -- The revision that replaced this rejected one.
    resubmitted_as INTEGER,
    status TEXT NOT NULL DEFAULT 'draft',
    -- The extension of time: the days asked and granted, the delay event and
    -- its date.
    days_asked INTEGER,
    days_granted INTEGER,
    event TEXT,
    event_date TEXT,
    -- The day it was submitted to the client, and whether that was after the
    -- contract's notice deadline.
    submitted_on TEXT,
    notice_date TEXT,
    notice_late INTEGER NOT NULL DEFAULT 0,
    -- The IPA that valued an approved variation.
    ipa_id INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_variations_project" ON "construction_variations" (project_id);
