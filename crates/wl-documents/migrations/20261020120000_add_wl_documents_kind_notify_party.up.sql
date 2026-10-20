-- Whether the party the document is about is reminded as well when it is about
-- to expire (wl-documents §5.2 `DOC.RUL.01`: a tenant's Emirates ID). Off for
-- the kinds that exist.
ALTER TABLE "wl_documents_kinds" ADD COLUMN notify_party INTEGER NOT NULL DEFAULT 0;
