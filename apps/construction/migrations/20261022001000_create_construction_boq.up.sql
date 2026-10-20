-- The BOQ lines a variation can omit work from, and the cumulative progress
-- an IPA records on them (02 Construction §4.5, §7.1 *Variation* `submit`;
-- BL-CON-12). Quantities are whole units of the line's unit; money is fils.
CREATE TABLE "construction_boq_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES "construction_projects" (id),
    -- The line number in the BOQ, e.g. `4.2`.
    number TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    unit TEXT NOT NULL DEFAULT '',
    -- The quantity of the contract, and the rate per unit.
    quantity INTEGER NOT NULL DEFAULT 0,
    rate INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_boq_lines_project" ON "construction_boq_lines" (project_id);

CREATE TABLE "construction_ipa_progress" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ipa_id INTEGER NOT NULL REFERENCES "construction_ipas" (id),
    boq_line_id INTEGER NOT NULL REFERENCES "construction_boq_lines" (id),
    -- The cumulative quantity done on the line, as the IPA records it.
    quantity INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "construction_ipa_progress_ipa" ON "construction_ipa_progress" (ipa_id);

-- An omission takes a quantity out of a BOQ line; the excess is what was
-- already done when the client approved it.
ALTER TABLE "construction_variations" ADD COLUMN boq_line_id INTEGER;
ALTER TABLE "construction_variations" ADD COLUMN omitted_quantity INTEGER;
ALTER TABLE "construction_variations" ADD COLUMN excess_quantity INTEGER NOT NULL DEFAULT 0;
