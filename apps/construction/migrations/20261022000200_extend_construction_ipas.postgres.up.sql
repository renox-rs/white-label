-- The IPA's place in the monthly cycle (02 Construction §4.4, §7.1 *IPA*,
-- `CON.RUL.03`): its number in the project, the replacement chain, the
-- certificate and the claim gap. Money is fils.
-- The sequence number within the project, from 1 (0: not numbered).
ALTER TABLE "construction_ipas" ADD COLUMN number BIGINT NOT NULL DEFAULT 0;
-- A draft made while an earlier IPA is still waiting for its certificate.
ALTER TABLE "construction_ipas" ADD COLUMN previous_pending BOOLEAN NOT NULL DEFAULT FALSE;
-- The cumulative certified amount of the latest certified or invoiced IPA.
ALTER TABLE "construction_ipas" ADD COLUMN previous_certified BIGINT NOT NULL DEFAULT 0;
-- The pending IPA this one replaces (no foreign key: it stays a plain number).
ALTER TABLE "construction_ipas" ADD COLUMN replaces_id BIGINT;
-- The cut-off day a monthly draft was made for.
ALTER TABLE "construction_ipas" ADD COLUMN cutoff_on DATE;
ALTER TABLE "construction_ipas" ADD COLUMN submitted_on DATE;
ALTER TABLE "construction_ipas" ADD COLUMN certified_on DATE;
-- The client's certificate, a document reference.
ALTER TABLE "construction_ipas" ADD COLUMN certificate_ref TEXT;
-- Claimed less certified work, recorded at `certify`.
ALTER TABLE "construction_ipas" ADD COLUMN claim_gap BIGINT;
-- A certificate for a superseded IPA that arrived when the replacing IPA was
-- certified already: kept for reference, no amount changes.
ALTER TABLE "construction_ipas" ADD COLUMN late_certificate_ref TEXT;

-- One monthly draft per project per cut-off.
CREATE UNIQUE INDEX "construction_ipas_cutoff"
    ON "construction_ipas" (project_id, cutoff_on)
    WHERE cutoff_on IS NOT NULL;
