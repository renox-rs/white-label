-- 05 Clinics payer contract (`CLN.INS.01`; base for charge capture, BL-CLN-05,
-- BL-CLN-06): what a payer pays for the plan of a policy: the share the
-- patient bears (copay in basis points of the covered amount, a deductible
-- per visit), the id of the payer agreement in wl-invoicing the claims are
-- billed under, and a tariff per service (the payer's price, or the service
-- excluded from the cover).
CREATE TABLE "clinics_payer_contracts" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payer_party_id INTEGER NOT NULL,
    plan TEXT,
    agreement_id INTEGER,
    copay_bp INTEGER NOT NULL DEFAULT 0,
    deductible INTEGER NOT NULL DEFAULT 0,
    valid_from TEXT NOT NULL,
    valid_to TEXT,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE INDEX "clinics_payer_contracts_payer_party_id" ON "clinics_payer_contracts" (payer_party_id);

CREATE TABLE "clinics_payer_tariffs" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contract_id INTEGER NOT NULL REFERENCES "clinics_payer_contracts" (id),
    service_code TEXT NOT NULL,
    price INTEGER,
    covered BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "clinics_payer_tariffs_contract_id_service_code" ON "clinics_payer_tariffs" (contract_id, service_code);
