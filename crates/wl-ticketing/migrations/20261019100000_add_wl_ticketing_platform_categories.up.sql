-- Platform categories (wl-ticketing §7, issue #156): categories other modules
-- open tickets in. A client may rename them but not delete or turn them off.
ALTER TABLE "wl_ticketing_categories" ADD COLUMN platform INTEGER NOT NULL DEFAULT 0;
-- The role whose holders look after the tickets of the category, e.g. `hr`.
ALTER TABLE "wl_ticketing_categories" ADD COLUMN default_assignee_role TEXT;
-- The custom fields of the category's tickets: a JSON list of
-- `{key, label_en, label_ar, kind, required, choices}`.
ALTER TABLE "wl_ticketing_categories" ADD COLUMN custom_fields TEXT NOT NULL DEFAULT '[]';

UPDATE "wl_ticketing_categories" SET platform = 1 WHERE key IN ('task', 'billing_dispute');
