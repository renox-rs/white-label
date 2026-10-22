-- 10 EdTech instalment (`FIN.BIL.01`, 10 section 4.3): the schedule of an
-- enrolment's course fee. The first is invoiced with the enrolment, the
-- others on their due dates; `withdraw` cancels the ones not yet invoiced.
CREATE TABLE "edtech_instalments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    enrolment_id INTEGER NOT NULL REFERENCES "edtech_enrolments" (id),
    number INTEGER NOT NULL,
    due_date TEXT NOT NULL,
    amount INTEGER NOT NULL,
    invoice_id INTEGER,
    status TEXT NOT NULL DEFAULT 'scheduled',
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_instalments_enrolment_id_number" ON "edtech_instalments" (enrolment_id, number);
CREATE INDEX "edtech_instalments_due_date" ON "edtech_instalments" (due_date);
