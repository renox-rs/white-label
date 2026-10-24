-- 08 Auto insurers (`AUT.INS.01`, 08 section 6 *Insurer*): the terms the
-- workshop has with an insurance company, which is a party. The payer
-- agreement is the id of the agreement in wl-invoicing the payer bills are
-- made under (`FIN.TPP.01`); the rate card of the insurer (`FIN.RAT.01`) is
-- kept by wl-invoicing for the source `auto.insurer:N`.
CREATE TABLE "auto_insurers" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    party_id INTEGER NOT NULL,
    payer_agreement_id INTEGER,
    claim_email TEXT,
    surveyor_contacts TEXT,
    payment_terms_days INTEGER NOT NULL DEFAULT 30,
    labour_rate INTEGER,
    storage_rate_daily INTEGER,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_by INTEGER,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "auto_insurers_party_id" ON "auto_insurers" (party_id);
CREATE INDEX "auto_insurers_active" ON "auto_insurers" (active);
