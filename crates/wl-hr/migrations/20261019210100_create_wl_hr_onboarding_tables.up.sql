-- Onboarding templates and their steps (wl-hr §3 *Onboarding template*,
-- issue #216).
CREATE TABLE "wl_hr_onboarding_templates" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    -- Applies to every employee.
    all_staff INTEGER NOT NULL DEFAULT 0,
    -- Comma-separated job role or trade keys, lower case; empty with
    -- all_staff.
    job_roles TEXT NOT NULL DEFAULT '',
    -- Comma-separated employment types; empty for every type.
    employment_types TEXT NOT NULL DEFAULT '',
    -- Activation also needs the required steps of this template (BL-HRM-29).
    blocks_activation INTEGER NOT NULL DEFAULT 0,
    -- Counts the changes of the template and its steps.
    version INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE "wl_hr_onboarding_template_steps" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL REFERENCES "wl_hr_onboarding_templates" (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    title_en TEXT NOT NULL,
    title_ar TEXT NOT NULL DEFAULT '',
    -- `hr`, `manager`, `employee` or `role:<name>`.
    owner TEXT NOT NULL DEFAULT 'hr',
    -- Days after the hire date (the start date of a manual onboarding).
    due_days INTEGER NOT NULL DEFAULT 0,
    required INTEGER NOT NULL DEFAULT 1,
    -- `task`, `checklist`, `document` or `sop_acknowledgement`.
    kind TEXT NOT NULL DEFAULT 'task',
    checklist_template TEXT,
    document_kind TEXT,
    evidence_required INTEGER NOT NULL DEFAULT 0,
    help_text TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (template_id, position)
);

-- One employee's onboarding. The status moves only through the Onboarding
-- machine.
CREATE TABLE "wl_hr_onboardings" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    employee_id INTEGER NOT NULL REFERENCES "wl_hr_employees" (id),
    -- `hire` or `manual`.
    start_trigger TEXT NOT NULL DEFAULT 'hire',
    -- The date the due days count from.
    started_on TEXT NOT NULL,
    -- Why it was cancelled.
    cancel_reason TEXT,
    status TEXT NOT NULL DEFAULT 'in_progress',
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_onboardings_employee" ON "wl_hr_onboardings" (employee_id, status);

-- The templates an onboarding was started from, at the version frozen then.
CREATE TABLE "wl_hr_onboarding_templates_applied" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    onboarding_id INTEGER NOT NULL REFERENCES "wl_hr_onboardings" (id) ON DELETE CASCADE,
    template_id INTEGER NOT NULL,
    template_version INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (onboarding_id, template_id)
);

-- One step for one employee; the template step is copied, so a later edit of
-- the template does not change a running onboarding. The status moves only
-- through the OnboardingStep machine.
CREATE TABLE "wl_hr_onboarding_steps" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    onboarding_id INTEGER NOT NULL REFERENCES "wl_hr_onboardings" (id) ON DELETE CASCADE,
    template_id INTEGER NOT NULL,
    template_step_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    title_en TEXT NOT NULL,
    title_ar TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL,
    kind TEXT NOT NULL,
    required INTEGER NOT NULL DEFAULT 1,
    evidence_required INTEGER NOT NULL DEFAULT 0,
    checklist_template TEXT,
    document_kind TEXT,
    help_text TEXT NOT NULL DEFAULT '',
    due_days INTEGER NOT NULL DEFAULT 0,
    due_date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    done_by INTEGER,
    done_at TEXT,
    evidence_note TEXT,
    evidence_document_id INTEGER,
    checklist_run_id INTEGER,
    -- The items of the last failed checklist run.
    failed_items TEXT,
    -- A step recorded in person: the staff user and the drawn signature.
    witness_id INTEGER,
    signature TEXT,
    waived_by INTEGER,
    waive_reason TEXT,
    -- Noted when the step was done by the system, e.g. *no SOPs*.
    note TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "wl_hr_onboarding_steps_onboarding" ON "wl_hr_onboarding_steps" (onboarding_id, status);

-- The users a step is assigned to: all of them, any one marks it done.
CREATE TABLE "wl_hr_onboarding_step_assignees" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    step_id INTEGER NOT NULL REFERENCES "wl_hr_onboarding_steps" (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT,
    UNIQUE (step_id, user_id)
);
