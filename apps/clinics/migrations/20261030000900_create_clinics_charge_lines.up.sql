-- 05 Clinics charge capture (`CLN.ENC.01`, `CLN.ENC.02`; base for BL-CLN-05,
-- BL-CLN-06): the services delivered and the consumables issued on a visit,
-- each priced and split between the patient and the payer when the visit is
-- billed; and what the visit records of its work done and of its bill.
CREATE TABLE "clinics_charge_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    visit_id INTEGER NOT NULL REFERENCES "clinics_visits" (id),
    kind TEXT NOT NULL,
    code TEXT NOT NULL,
    units INTEGER NOT NULL,
    unit_price INTEGER NOT NULL DEFAULT 0,
    list_price INTEGER NOT NULL DEFAULT 0,
    self_pay_units INTEGER NOT NULL DEFAULT 0,
    self_pay_price INTEGER NOT NULL DEFAULT 0,
    authorised_units INTEGER NOT NULL DEFAULT 0,
    vat_standard BOOLEAN NOT NULL DEFAULT FALSE,
    patient_share INTEGER NOT NULL DEFAULT 0,
    payer_share INTEGER NOT NULL DEFAULT 0,
    reason TEXT,
    package_id INTEGER,
    removed_on TEXT,
    removed_reason TEXT,
    recorded_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_charge_lines_visit_id" ON "clinics_charge_lines" (visit_id);

ALTER TABLE "clinics_visits" ADD COLUMN work_none BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "clinics_visits" ADD COLUMN work_accepted_by INTEGER;
ALTER TABLE "clinics_visits" ADD COLUMN work_accepted_at TEXT;
ALTER TABLE "clinics_visits" ADD COLUMN not_suitable BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "clinics_visits" ADD COLUMN deposit_refund_full BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "clinics_visits" ADD COLUMN billed_at TEXT;
ALTER TABLE "clinics_visits" ADD COLUMN invoice_id INTEGER;
ALTER TABLE "clinics_visits" ADD COLUMN payer_bill_id INTEGER;
ALTER TABLE "clinics_visits" ADD COLUMN authorisations_consumed_at TEXT;
ALTER TABLE "clinics_visits" ADD COLUMN deposit_settled_at TEXT;
ALTER TABLE "clinics_visits" ADD COLUMN patient_total INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "clinics_visits" ADD COLUMN payer_total INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "clinics_visits" ADD COLUMN deposit_applied INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "clinics_visits" ADD COLUMN deposit_refunded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "clinics_visits" ADD COLUMN clinic_stop BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE "clinics_visits" ADD COLUMN scheduling_applied INTEGER NOT NULL DEFAULT 0;
