ALTER TABLE "accounting_tax_profiles" DROP COLUMN notes;
ALTER TABLE "accounting_tax_profiles" DROP COLUMN contact_email;
DROP INDEX IF EXISTS "accounting_tax_profile_changes_one_pending";
DROP INDEX IF EXISTS "accounting_tax_profile_changes_status";
DROP INDEX IF EXISTS "accounting_tax_profile_changes_client_party_id";
DROP TABLE IF EXISTS "accounting_tax_profile_changes";
