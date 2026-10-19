DROP INDEX IF EXISTS "wl_parties_sanctions_list_uploads_list";
DROP TABLE IF EXISTS "wl_parties_sanctions_list_uploads";
ALTER TABLE "wl_parties_sanctions_lists" DROP COLUMN format;
ALTER TABLE "wl_parties_sanctions_lists" DROP COLUMN source_url;
ALTER TABLE "wl_parties_sanctions_lists" DROP COLUMN updated_by;
