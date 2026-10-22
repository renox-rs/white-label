-- 10 EdTech chargeback (10 section 4.3 *A card payment is charged back*,
-- BL-EDU-06): an invoice of an enrolment that was paid and reopened by a
-- reversed payment, with the balance it came back with. The counsellor has one
-- task per enrolment while a row is unsettled; a row is settled when the invoice
-- is paid again.
CREATE TABLE "edtech_chargebacks" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    enrolment_id INTEGER NOT NULL REFERENCES "edtech_enrolments" (id),
    invoice_id INTEGER NOT NULL,
    amount INTEGER NOT NULL DEFAULT 0,
    reversal_reason TEXT,
    reopened_on TEXT NOT NULL,
    settled_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "edtech_chargebacks_invoice_id" ON "edtech_chargebacks" (invoice_id);
CREATE INDEX "edtech_chargebacks_enrolment_id" ON "edtech_chargebacks" (enrolment_id);
