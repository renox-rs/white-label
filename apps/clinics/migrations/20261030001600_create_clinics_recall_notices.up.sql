-- 05 Clinics, product recall (BL-CLN-14, #362): the notice a patient (or the
-- guardian of a dependant) gets about a recalled product, one per patient and
-- recall, with the visits it covers, the lines of the recall it tells, and
-- what happened to it: sent, gave up (a call task for the front desk) or
-- called (the outcome of the call). The text never names the product.
CREATE TABLE "clinics_recall_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recall_id INTEGER NOT NULL,
    party_id INTEGER NOT NULL,
    patient_party_id INTEGER,
    visit_ids TEXT NOT NULL DEFAULT '[]',
    line_ids TEXT NOT NULL DEFAULT '[]',
    visit_dates TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    sent_by INTEGER,
    sent_at TEXT,
    gave_up_at TEXT,
    call_outcome TEXT,
    called_by INTEGER,
    called_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_recall_notices_recall_id" ON "clinics_recall_notices" (recall_id);
CREATE INDEX "clinics_recall_notices_status" ON "clinics_recall_notices" (status);
