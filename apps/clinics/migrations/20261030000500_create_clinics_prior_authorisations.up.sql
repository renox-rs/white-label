-- 05 Clinics prior authorisation (`CLN.INS.03`; base for BL-CLN-02, BL-CLN-03):
-- a payer's approval for named services and a number of units, valid to a
-- date, with the machine's status; the payer's list of services that need
-- one (the prior-auth list of the payer contract, `CLN.INS.01`, minimal);
-- the units a checked-in visit holds against an authorisation until the
-- visit is completed or cancelled.
CREATE TABLE "clinics_prior_authorisations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    policy_id INTEGER NOT NULL REFERENCES "clinics_insurance_policies" (id),
    patient_party_id INTEGER NOT NULL,
    payer_party_id INTEGER NOT NULL,
    services TEXT NOT NULL DEFAULT '[]',
    units_requested INTEGER NOT NULL,
    units_approved INTEGER NOT NULL DEFAULT 0,
    units_consumed INTEGER NOT NULL DEFAULT 0,
    reference TEXT,
    valid_until TEXT,
    denial_reason TEXT,
    requested_on TEXT,
    decided_on TEXT,
    decided_by INTEGER,
    created_by INTEGER,
    status TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_prior_authorisations_patient_party_id" ON "clinics_prior_authorisations" (patient_party_id);
CREATE INDEX "clinics_prior_authorisations_status" ON "clinics_prior_authorisations" (status);

CREATE TABLE "clinics_prior_auth_services" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_party_id INTEGER NOT NULL,
    service_code TEXT NOT NULL,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_prior_auth_services_payer_party_id_service_code" ON "clinics_prior_auth_services" (payer_party_id, service_code);

CREATE TABLE "clinics_authorisation_holds" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    authorisation_id INTEGER NOT NULL REFERENCES "clinics_prior_authorisations" (id),
    visit_id INTEGER NOT NULL REFERENCES "clinics_visits" (id),
    booking_id INTEGER NOT NULL,
    service_code TEXT NOT NULL,
    units INTEGER NOT NULL,
    released_on TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_authorisation_holds_authorisation_id" ON "clinics_authorisation_holds" (authorisation_id);
CREATE INDEX "clinics_authorisation_holds_visit_id" ON "clinics_authorisation_holds" (visit_id);
