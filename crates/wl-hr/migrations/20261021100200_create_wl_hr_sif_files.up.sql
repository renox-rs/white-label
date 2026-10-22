-- SIF files of a payroll run and the WPS rejection (wl-hr §3 *Payroll run*:
-- SIF files, §4 `record_rejection` and `resubmit`, `HRM.WPS.01`, issue #202).

-- One row per SIF version generated for a run; an earlier version is never
-- overwritten or deleted. `establishment_id` is the MoHRE establishment ID
-- the file was named with.
CREATE TABLE "wl_hr_sif_files" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES "wl_hr_payroll_runs" (id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    establishment_id TEXT,
    file_name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    line_count INTEGER NOT NULL DEFAULT 0,
    -- The sum of the nets, AED fils.
    total INTEGER NOT NULL DEFAULT 0,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (run_id, version)
);

-- The lines a SIF version carries, with the net they were sent with: a
-- resubmit sends the same lines with the same nets.
CREATE TABLE "wl_hr_sif_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sif_file_id INTEGER NOT NULL REFERENCES "wl_hr_sif_files" (id) ON DELETE CASCADE,
    line_id INTEGER NOT NULL REFERENCES "wl_hr_payroll_lines" (id),
    employee_id INTEGER NOT NULL,
    net_amount INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_sif_lines_file" ON "wl_hr_sif_lines" (sif_file_id);
CREATE INDEX "wl_hr_sif_lines_line" ON "wl_hr_sif_lines" (line_id);

-- The MoHRE establishment ID of a legal entity (each entity is its own
-- establishment, decision 0003); the SIF file is named with it.
CREATE TABLE "wl_hr_establishments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    legal_entity_id INTEGER NOT NULL UNIQUE,
    mohre_establishment_id TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

-- The rejection of the whole file: the reason (`file_format`,
-- `establishment_id`, `funding`, `other`), the bank's notice, and the note on
-- the fix entered at `resubmit`.
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN rejection_reason TEXT;
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN rejection_notice TEXT;
ALTER TABLE "wl_hr_payroll_runs" ADD COLUMN resubmit_note TEXT;
