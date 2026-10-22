-- 10 EdTech waiting list of a batch (`EDU.ACL.17`, 10 section 4.1 *Two people
-- take the last seat*): a plain list of the students who asked for a seat of a
-- full batch (student, payer, created at). No seat is held and nothing is
-- offered when a seat frees; counsellors see the list.
CREATE TABLE "edtech_batch_waiting" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    batch_id INTEGER NOT NULL REFERENCES "edtech_batches" (id),
    student_party_id INTEGER NOT NULL,
    payer_party_id INTEGER NOT NULL,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_batch_waiting_batch_id_student_party_id" ON "edtech_batch_waiting" (batch_id, student_party_id);
