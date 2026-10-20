ALTER TABLE "accounting_engagements" DROP COLUMN decline_note;
DROP INDEX IF EXISTS "accounting_engagement_letters_engagement_id";
DROP TABLE IF EXISTS "accounting_engagement_letters";
