DROP TABLE IF EXISTS "wl_documents_renewal_tasks";
DROP TABLE IF EXISTS "wl_documents_expired_logs";
ALTER TABLE "wl_documents_kinds" DROP COLUMN also_notify;
