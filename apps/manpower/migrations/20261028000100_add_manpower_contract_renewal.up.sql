-- The renewal of a client contract and the warnings before it ends (09
-- Manpower §7.1 *Client contract* `renew`, §7.2 `MPW.RUL.11`, BL-MPW-05):
-- the signed renewal, the end date it replaced and the rate card version
-- that changed the rates, and one row per notice sent so that each threshold
-- is told once per end date.
ALTER TABLE "manpower_client_contracts" ADD COLUMN renewed_at TEXT;
ALTER TABLE "manpower_client_contracts" ADD COLUMN renewed_by INTEGER;
-- The signed renewal, a document reference (`DOC.SIG.01`), and the day it was signed.
ALTER TABLE "manpower_client_contracts" ADD COLUMN renewal_document TEXT;
ALTER TABLE "manpower_client_contracts" ADD COLUMN renewal_signed_on TEXT;
-- The end date before the latest renewal.
ALTER TABLE "manpower_client_contracts" ADD COLUMN renewal_previous_end_date TEXT;
-- The effective date of the rate card version the renewal changed the rates with.
ALTER TABLE "manpower_client_contracts" ADD COLUMN renewal_rates_from TEXT;

CREATE TABLE "manpower_contract_end_notices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_contract_id INTEGER NOT NULL REFERENCES "manpower_client_contracts" (id),
    -- The end date the notice was about: a renewal starts the notices again.
    end_date TEXT NOT NULL,
    -- The threshold: 60 or 30 days before the end date.
    days_before INTEGER NOT NULL,
    sent_at TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE UNIQUE INDEX "manpower_contract_end_notices_once" ON "manpower_contract_end_notices" (client_contract_id, end_date, days_before);
