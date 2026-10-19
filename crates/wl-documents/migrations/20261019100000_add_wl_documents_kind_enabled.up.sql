-- Whether new documents of the kind may be attached (wl-documents §3, §7):
-- a kind that is not enabled is available to the client and off. Existing
-- documents stay and their reminders continue. Kinds that exist are on.
ALTER TABLE "wl_documents_kinds" ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
