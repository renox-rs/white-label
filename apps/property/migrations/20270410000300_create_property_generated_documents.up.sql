-- The documents 01 generates (01 Property §7.1 *Lease* `submit`, *Owner
-- statement* `publish`, `DOC.GEN.01`; BL-PRP-31): one per record and
-- template, so a submitted version's contract or a published statement's PDF
-- is never generated again.
CREATE TABLE "property_generated_documents" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL,
    template TEXT NOT NULL,
    title TEXT NOT NULL,
    document_id INTEGER NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "property_generated_documents_subject_template" ON "property_generated_documents" (subject_type, subject_id, template);
