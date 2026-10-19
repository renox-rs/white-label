-- A short label for the subject in staff-facing text, e.g. `TKT-2026-0042`,
-- from the request; none falls back to the subject's own text.
ALTER TABLE "wl_notify_messages" ADD COLUMN subject_label TEXT;
